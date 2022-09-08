//! IPC handling

use crate::{error::RequestError, pubsub::PubSubConnection, PubSubRpcHandler};
use anvil_rpc::request::Request;
use bytes::BytesMut;
use futures::{ready, Sink, Stream, StreamExt};
use parity_tokio_ipc::Endpoint;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tracing::{error, trace};

/// An IPC connection for anvil
///
/// A Future that listens for incoming connections and spawns new connections
pub struct IpcEndpoint<Handler> {
    /// the handler for the websocket connection
    handler: Handler,
    /// The endpoint we listen for incoming transactions
    endpoint: Endpoint,
    // TODO add shutdown
}

impl<Handler: PubSubRpcHandler> IpcEndpoint<Handler> {
    /// Creates a new endpoint with the given handler
    pub fn new(handler: Handler, endpoint: Endpoint) -> Self {
        Self { handler, endpoint }
    }

    /// Start listening for incoming connections
    pub async fn listen(self) {
        let IpcEndpoint { handler, endpoint } = self;
        trace!(target: "ipc",  endpoint=?endpoint.path(), "starting ipc server" );

        let mut connections = match endpoint.incoming() {
            Ok(connections) => connections,
            Err(err) => {
                error!(target: "ipc",  ?err, "Failed to create ipc listener");
                return
            }
        };

        while let Some(Ok(stream)) = connections.next().await {
            trace!(target: "ipc", "successful incoming IPC connection");

            let framed = tokio_util::codec::Decoder::framed(JsonRpcCodec, stream);
            let conn = PubSubConnection::new(IpcConn(framed), handler.clone());

            // spawn the new connection
            tokio::task::spawn(async move { conn.await });
        }
    }
}

#[pin_project::pin_project]
struct IpcConn<T>(#[pin] T);

impl<T> Stream for IpcConn<T>
where
    T: Stream<Item = io::Result<String>>,
{
    type Item = Result<Option<Request>, RequestError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        fn on_request(msg: io::Result<String>) -> Result<Option<Request>, RequestError> {
            let text = msg?;
            Ok(Some(serde_json::from_str(&text)?))
        }
        match ready!(self.project().0.poll_next(cx)) {
            Some(req) => Poll::Ready(Some(on_request(req))),
            _ => Poll::Ready(None),
        }
    }
}

impl<T> Sink<String> for IpcConn<T>
where
    T: Sink<String, Error = io::Error>,
{
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().0.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: String) -> Result<(), Self::Error> {
        self.project().0.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().0.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().0.poll_close(cx)
    }
}

struct JsonRpcCodec;

impl tokio_util::codec::Decoder for JsonRpcCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        const fn is_whitespace(byte: u8) -> bool {
            matches!(byte, 0x0D | 0x0A | 0x20 | 0x09)
        }

        let mut depth = 0;
        let mut in_str = false;
        let mut is_escaped = false;
        let mut start_idx = 0;
        let mut whitespaces = 0;

        for idx in 0..buf.as_ref().len() {
            let byte = buf.as_ref()[idx];

            if (byte == b'{' || byte == b'[') && !in_str {
                if depth == 0 {
                    start_idx = idx;
                }
                depth += 1;
            } else if (byte == b'}' || byte == b']') && !in_str {
                depth -= 1;
            } else if byte == b'"' && !is_escaped {
                in_str = !in_str;
            } else if is_whitespace(byte) {
                whitespaces += 1;
            }
            if byte == b'\\' && !is_escaped && in_str {
                is_escaped = true;
            } else {
                is_escaped = false;
            }

            if depth == 0 && idx != start_idx && idx - start_idx + 1 > whitespaces {
                let bts = buf.split_to(idx + 1);
                match String::from_utf8(bts.as_ref().to_vec()) {
                    Ok(val) => return Ok(Some(val)),
                    Err(_) => return Ok(None),
                };
            }
        }
        Ok(None)
    }
}

impl tokio_util::codec::Encoder<String> for JsonRpcCodec {
    type Error = io::Error;

    fn encode(&mut self, msg: String, buf: &mut BytesMut) -> io::Result<()> {
        buf.extend_from_slice(msg.as_bytes());
        Ok(())
    }
}
