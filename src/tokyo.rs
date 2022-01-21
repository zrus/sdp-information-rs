use std::time::Instant;

use bytes::{Bytes, BytesMut, Buf, BufMut};
use futures::{Stream, Sink, SinkExt, StreamExt};
use pretty_hex::PrettyHex;
use rtsp_connection::{wrap, ConnectionContext, ReceivedMessage, RtspMessageContext, WallTime};
use rtsp_types::{Data, Message};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;
use url::Host;

use crate::error::{Error, ErrorInt};

struct Codec {
    ctx: ConnectionContext,
    read_pos: u64,
}

enum CodecError {
    IoError(std::io::Error),
    ParseError { description: String, pos: u64 },
}

impl std::convert::From<std::io::Error> for CodecError {
    fn from(e: std::io::Error) -> Self {
        CodecError::IoError(e)
    }
}

impl Codec {
    fn parse_msg(&self, src: &mut BytesMut) -> Result<Option<(usize, Message<Bytes>)>, CodecError> {
        if !src.is_empty() && src[0] == b'$' {
            if src.len() < 4 {
                return Ok(None);
            }
            let channel_id = src[1];
            let len = 4 + usize::from(u16::from_be_bytes([src[2], src[3]]));
            if src.len() < len {
                src.reserve(len - src.len());
                return Ok(None);
            }
            let mut msg = src.split_to(len);
            msg.advance(4);
            return Ok(Some((
                len,
                Message::Data(Data::new(channel_id, msg.freeze())),
            )));
        }

        let (msg, len): (Message<&[u8]>, _) = match Message::parse(src) {
            Ok((m, l)) => (m, l),
            Err(rtsp_types::ParseError::Error) => {
                const MAX_DUMP_BYTES: usize = 128;
                let conf = pretty_hex::HexConfig {
                    title: false,
                    ..Default::default()
                };
                if src.len() > MAX_DUMP_BYTES {
                    return Err(CodecError::ParseError {
                        description: format!(
                            "Invalid RTSP message; next {} of {} buffered bytes are:\n{:#?}",
                            MAX_DUMP_BYTES,
                            src.len(),
                            (&src[0..MAX_DUMP_BYTES]).hex_conf(conf)
                        ),
                        pos: self.read_pos,
                    });
                }
                return Err(CodecError::ParseError {
                    description: format!(
                        "Invalid RTSP message; next {} buffered bytes are:\n{:#?}",
                        MAX_DUMP_BYTES,
                        src.hex_conf(conf)
                    ),
                    pos: self.read_pos,
                });
            }
            Err(rtsp_types::ParseError::Incomplete) => return Ok(None),
        };

        let msg = match msg {
            Message::Request(msg) => {
                let body_range = rtsp_connection::as_range(src, msg.body());
                let msg = msg.replace_body(rtsp_types::Empty);
                if let Some(r) = body_range {
                    let mut raw_msg = src.split_to(len);
                    raw_msg.advance(r.start);
                    raw_msg.truncate(r.len());
                    Message::Request(msg.replace_body(raw_msg.freeze()))
                } else {
                    src.advance(len);
                    Message::Request(msg.replace_body(Bytes::new()))
                }
            }
            Message::Response(msg) => {
                let body_range = rtsp_connection::as_range(src, msg.body());
                let msg = msg.replace_body(rtsp_types::Empty);
                if let Some(r) = body_range {
                    let mut raw_msg = src.split_to(len);
                    raw_msg.advance(r.start);
                    raw_msg.truncate(r.len());
                    Message::Response(msg.replace_body(raw_msg.freeze()))
                } else {
                    src.advance(len);
                    Message::Response(msg.replace_body(Bytes::new()))
                }
            }
            Message::Data(_) => unreachable!(),
        };
        Ok(Some((len, msg)))
    }
}

impl tokio_util::codec::Decoder for Codec {
    type Item = ReceivedMessage;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let (len, msg) = match self.parse_msg(src) {
            Err(e) => return Err(e),
            Ok(None) => return Ok(None),
            Ok(Some((len, msg))) => (len, msg),
        };
        let msg = ReceivedMessage {
            msg,
            ctx: RtspMessageContext {
                pos: self.read_pos,
                received_wall: WallTime::now(),
                received: Instant::now(),
            },
        };
        self.read_pos += u64::try_from(len).expect("usize fits in u64");
        Ok(Some(msg))
    }
}

impl tokio_util::codec::Encoder<rtsp_types::Message<Bytes>> for Codec {
    type Error = CodecError;

    fn encode(
        &mut self,
        item: rtsp_types::Message<Bytes>,
        mut dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        item.write(&mut (&mut dst).writer())
            .expect("BufMut Writer is infallible");
        Ok(())
    }
}

pub struct Connection(Framed<TcpStream, Codec>);
impl Connection {
    pub async fn connect(host: Host<&str>, port: u16) -> Result<Self, std::io::Error> {
        let stream = match host {
            Host::Domain(h) => TcpStream::connect((h, port)).await,
            Host::Ipv4(h) => TcpStream::connect((h, port)).await,
            Host::Ipv6(h) => TcpStream::connect((h, port)).await,
        }?;
        Self::from_stream(stream)
    }

    pub fn from_stream(stream: TcpStream) -> Result<Self, std::io::Error> {
        let established_wall = WallTime::now();
        let established = Instant::now();
        let local_addr = stream.local_addr()?;
        let peer_addr = stream.peer_addr()?;
        Ok(Self(Framed::new(
            stream,
            Codec {
                ctx: ConnectionContext {
                    local_addr,
                    peer_addr,
                    established_wall,
                    established,
                },
                read_pos: 0,
            },
        )))
    }

    pub(crate) fn ctx(&self) -> &ConnectionContext {
        &self.0.codec().ctx
    }

    pub(crate) fn eof_ctx(&self) -> RtspMessageContext {
        RtspMessageContext {
            pos: self.0.codec().read_pos
                + u64::try_from(self.0.read_buffer().remaining()).expect("usize fits in u64"),
            received_wall: WallTime::now(),
            received: Instant::now(),
        }
    }

    fn wrap_write_err(&self, e: CodecError) -> ErrorInt {
        match e {
            CodecError::IoError(source) => ErrorInt::WriteError {
                conn_ctx: *self.ctx(),
                source,
            },
            CodecError::ParseError { .. } => unreachable!(),
        }
    }
}

impl Stream for Connection {
    type Item = Result<ReceivedMessage, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_next_unpin(cx).map_err(|e| {
            wrap!(match e {
                CodecError::IoError(error) => ErrorInt::RtspReadError {
                    conn_ctx: *self.ctx(),
                    msg_ctx: self.eof_ctx(),
                    source: error,
                },
                CodecError::ParseError { description, pos } => ErrorInt::RtspFramingError {
                    conn_ctx: *self.ctx(),
                    msg_ctx: RtspMessageContext {
                        pos,
                        received_wall: WallTime::now(),
                        received: Instant::now(),
                    },
                    description,
                },
            })
        })
    }
}

impl Sink<Message<Bytes>> for Connection {
    type Error = ErrorInt;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.0
            .poll_ready_unpin(cx)
            .map_err(|e| self.wrap_write_err(e))
    }

    fn start_send(
        mut self: std::pin::Pin<&mut Self>,
        item: Message<Bytes>,
    ) -> Result<(), Self::Error> {
        self.0
            .start_send_unpin(item)
            .map_err(|e| self.wrap_write_err(e))
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.0
            .poll_flush_unpin(cx)
            .map_err(|e| self.wrap_write_err(e))
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.0
            .poll_close_unpin(cx)
            .map_err(|e| self.wrap_write_err(e))
    }
}
