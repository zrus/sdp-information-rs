use crate::{
    error::{Error, ErrorInt},
    tokyo,
};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rtsp_connection::{bail, wrap, RtspMessageContext};
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

pub struct RtspConnection {
    inner: tokyo::Connection,
    creds: Option<Credentials>,
    next_cseq: u32,
}

impl RtspConnection {
    pub fn creds(username: Option<String>, password: Option<String>) -> Option<Credentials> {
        match (username, password) {
            (Some(username), password) => Some(Credentials {
                username,
                password: password.unwrap_or_default(),
            }),
            (None, None) => None,
            _ => unreachable!(),
        }
    }

    pub async fn connect(url: &Url, creds: Option<Credentials>) -> Result<Self, Error> {
        let host =
            RtspConnection::validate_url(url).map_err(|e| wrap!(ErrorInt::InvalidArgument(e)))?;
        let port = url.port().unwrap_or(554);
        let inner = crate::tokyo::Connection::connect(host, port)
            .await
            .map_err(|e| wrap!(ErrorInt::ConnectError(e)))?;
        Ok(Self {
            inner,
            creds,
            next_cseq: 1,
        })
    }

    fn validate_url(url: &Url) -> Result<url::Host<&str>, String> {
        if url.scheme() != "rtsp" {
            return Err(format!(
                "Bad URL {}; only scheme rtsp supported",
                url.as_str()
            ));
        }
        if url.username() != "" || url.password().is_some() {
            return Err("URL must not contain credentials".to_owned());
        }
        url.host()
            .ok_or_else(|| format!("Must specify host in rtsp url {}", &url))
    }

    pub async fn get_sdp(
        &mut self,
        requested_auth: &mut Option<http_auth::PasswordClient>,
        req: &mut rtsp_types::Request<Bytes>,
    ) -> Result<(RtspMessageContext, u32, rtsp_types::Response<Bytes>), Error> {
        loop {
            let cseq = self.fill_req(requested_auth, req)?;
            self.inner
                .send(rtsp_types::Message::Request(req.clone()))
                .await
                .map_err(|e| wrap!(e))?;
            let (resp, msg_ctx) = loop {
                let msg = self.inner.next().await.unwrap()?;
                let msg_ctx = msg.ctx;
                match msg.msg {
                    rtsp_types::Message::Response(r) => {
                        if let Some(response_cseq) = get_cseq(&r) {
                            if response_cseq == cseq {
                                break (r, msg_ctx);
                            }
                        }
                    }
                    _ => continue,
                };
            };
            if resp.status() == rtsp_types::StatusCode::Unauthorized {
                if requested_auth.is_some() {
                    bail!(ErrorInt::RtspResponseError {
                        conn_ctx: *self.inner.ctx(),
                        msg_ctx,
                        method: req.method().clone(),
                        cseq,
                        status: resp.status(),
                        description: "Received Unauthorized after trying digest auth".into(),
                    })
                }
                let www_authenticate = match resp.header(&rtsp_types::headers::WWW_AUTHENTICATE) {
                    None => bail!(ErrorInt::RtspResponseError {
                        conn_ctx: *self.inner.ctx(),
                        msg_ctx,
                        method: req.method().clone(),
                        cseq,
                        status: resp.status(),
                        description: "Unauthorized without WWW-Authenticate header".into(),
                    }),
                    Some(h) => h,
                };
                if self.creds.is_none() {
                    bail!(ErrorInt::RtspResponseError {
                        conn_ctx: *self.inner.ctx(),
                        msg_ctx,
                        method: req.method().clone(),
                        cseq,
                        status: resp.status(),
                        description: "Authentication requested and no credentials supplied"
                            .to_owned(),
                    })
                }
                let www_authenticate = www_authenticate.as_str();
                *requested_auth = match http_auth::PasswordClient::try_from(www_authenticate) {
                    Ok(c) => Some(c),
                    Err(e) => bail!(ErrorInt::RtspResponseError {
                        conn_ctx: *self.inner.ctx(),
                        msg_ctx,
                        method: req.method().clone(),
                        cseq,
                        status: resp.status(),
                        description: format!("Can't understand WWW-Authenticate header: {}", e),
                    }),
                };
                continue;
            } else if !resp.status().is_success() {
                bail!(ErrorInt::RtspResponseError {
                    conn_ctx: *self.inner.ctx(),
                    msg_ctx,
                    method: req.method().clone(),
                    cseq,
                    status: resp.status(),
                    description: "Unexpected RTSP response status".into(),
                });
            }
            return Ok((msg_ctx, cseq, resp));
        }
    }

    fn fill_req(
        &mut self,
        requested_auth: &mut Option<http_auth::PasswordClient>,
        req: &mut rtsp_types::Request<Bytes>,
    ) -> Result<u32, Error> {
        let cseq = self.next_cseq;
        self.next_cseq += 1;
        if let Some(ref mut auth) = requested_auth {
            let creds = self
                .creds
                .as_ref()
                .expect("creds were checked when filling request_auth");
            let authorization = auth
                .respond(&http_auth::PasswordParams {
                    username: &creds.username,
                    password: &creds.password,
                    uri: req.request_uri().map(|u| u.as_str()).unwrap_or("*"),
                    method: req.method().into(),
                    body: Some(&[]),
                })
                .map_err(|e| wrap!(ErrorInt::Internal(e.into())))?;
            req.insert_header(rtsp_types::headers::AUTHORIZATION, authorization);
        }
        req.insert_header(rtsp_types::headers::CSEQ, cseq.to_string());
        Ok(cseq)
    }
}

fn get_cseq(response: &rtsp_types::Response<Bytes>) -> Option<u32> {
    response
        .header(&rtsp_types::headers::CSEQ)
        .and_then(|cseq| u32::from_str_radix(cseq.as_str(), 10).ok())
}
