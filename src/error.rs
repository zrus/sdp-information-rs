use std::{fmt::Display, sync::Arc};

use rtsp_connection::{ConnectionContext, RtspMessageContext};
use thiserror::Error;

#[derive(Clone)]
pub struct Error(pub(crate) Arc<ErrorInt>);

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Error)]
pub enum ErrorInt {
    /// The method's caller provided an invalid argument.
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    /// Unparseable or unexpected RTSP message.
    #[error("[{conn_ctx}, {msg_ctx}] RTSP framing error: {description}")]
    RtspFramingError {
        conn_ctx: ConnectionContext,
        msg_ctx: RtspMessageContext,
        description: String,
    },

    #[error("[{conn_ctx}, {msg_ctx}] {status} response to {} CSeq={cseq}: \
             {description}", Into::<&str>::into(.method))]
    RtspResponseError {
        conn_ctx: ConnectionContext,
        msg_ctx: RtspMessageContext,
        method: rtsp_types::Method,
        cseq: u32,
        status: rtsp_types::StatusCode,
        description: String,
    },

    #[error(
        "[{conn_ctx}, {msg_ctx}] Received interleaved data on unassigned channel {channel_id}"
    )]
    RtspUnassignedChannelError {
        conn_ctx: ConnectionContext,
        msg_ctx: RtspMessageContext,
        channel_id: u8,
    },

    #[error("Unable to connect to RTSP server: {0}")]
    ConnectError(#[source] std::io::Error),

    #[error("[{conn_ctx}, {msg_ctx}] Error reading from RTSP peer: {source}")]
    RtspReadError {
        conn_ctx: ConnectionContext,
        msg_ctx: RtspMessageContext,
        source: std::io::Error,
    },

    #[error("[{conn_ctx}] Error writing to RTSP peer: {source}")]
    WriteError {
        conn_ctx: ConnectionContext,
        source: std::io::Error,
    },

    #[error("Failed precondition: {0}")]
    FailedPrecondition(String),

    #[error("Internal error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("Timeout")]
    Timeout,
}
