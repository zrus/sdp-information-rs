use bytes::Bytes;
use client::RtspConnection;

pub mod client;
pub mod error;
pub mod tokyo;

use rtsp_types::{headers, Method, Version};
use url::Url;

#[tokio::main]
async fn main() {
    let url = Url::parse("rtsp://10.50.13.252/1/h264major").unwrap();

    let creds = RtspConnection::creds(None, None);
    let mut req = rtsp_types::Request::builder(Method::Describe, Version::V1_0)
        .header(headers::ACCEPT, "application/sdp")
        .request_uri(url.clone())
        .build(Bytes::new());
    let mut requested_auth = None;

    let (_, _, resp) = RtspConnection::connect(&url, creds)
        .await
        .unwrap()
        .get_sdp(&mut requested_auth, &mut req)
        .await
        .unwrap();

    println!("{:?}", resp.body());
}
