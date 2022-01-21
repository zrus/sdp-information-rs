use std::net::{IpAddr, SocketAddr};

use bytes::Bytes;
use rtsp_types::Message;

#[macro_export]
macro_rules! bail {
    ($e:expr) => {
        return Err(crate::error::Error(std::sync::Arc::new($e)))
    };
}

#[macro_export]
macro_rules! wrap {
    ($e:expr) => {
        crate::error::Error(std::sync::Arc::new($e))
    };
}

#[derive(Debug)]
pub struct ReceivedMessage {
    pub ctx: RtspMessageContext,
    pub msg: Message<Bytes>,
}

#[derive(Copy, Clone, Debug)]
pub struct WallTime(time::Timespec);

impl WallTime {
    pub fn now() -> Self {
        Self(time::get_time())
    }
}

impl std::fmt::Display for WallTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(
            &time::at(self.0)
                .strftime("%FT%T")
                .map_err(|_| std::fmt::Error)?,
            f,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ConnectionContext {
    pub local_addr: std::net::SocketAddr,
    pub peer_addr: std::net::SocketAddr,
    pub established_wall: WallTime,
    pub established: std::time::Instant,
}

impl ConnectionContext {
    #[doc(hidden)]
    pub fn dummy() -> Self {
        let addr = SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0);
        Self {
            local_addr: addr,
            peer_addr: addr,
            established_wall: WallTime::now(),
            established: std::time::Instant::now(),
        }
    }
}

impl std::fmt::Display for ConnectionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}(me)->{}@{}",
            &self.local_addr, &self.peer_addr, &self.established_wall,
        )
    }
}

#[derive(Copy, Clone, Debug)]
pub struct RtspMessageContext {
    pub pos: u64,
    pub received_wall: WallTime,
    pub received: std::time::Instant,
}

impl RtspMessageContext {
    #[doc(hidden)]
    pub fn dummy() -> Self {
        Self {
            pos: 0,
            received_wall: WallTime::now(),
            received: std::time::Instant::now(),
        }
    }

    pub fn received(&self) -> std::time::Instant {
        self.received
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }
}

impl std::fmt::Display for RtspMessageContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.pos, &self.received_wall)
    }
}

pub fn as_range(buf: &[u8], subset: &[u8]) -> Option<std::ops::Range<usize>> {
    if subset.is_empty() {
        return None;
    }
    let subset_p = subset.as_ptr() as usize;
    let buf_p = buf.as_ptr() as usize;
    let off = match subset_p.checked_sub(buf_p) {
        Some(off) => off,
        None => panic!(
            "{}-byte subset not within {}-byte buf",
            subset.len(),
            buf.len()
        ),
    };
    let end = off + subset.len();
    assert!(end <= buf.len());
    Some(off..end)
}
