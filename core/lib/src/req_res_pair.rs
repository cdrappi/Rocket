use std::convert::TryInto;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures_util::{future::BoxFuture, stream::Stream};
use tokio::io::AsyncRead;

use crate::{Rocket, Request};
use crate::response::{Body, Response};
use crate::http::hyper::{self, header, Bytes, HttpBody};
use crate::ext::{AsyncReadExt, IntoBytesStream};

/// Utility data structure for keeping a Response with the Request it might borrow data from
pub struct ReqResPair {
    rocket: Arc<Rocket>,
    // 'request' borrows from 'rocket'
    request: Option<Request<'static>>,
    // 'response' borrows from 'request'
    response: Option<Response<'static>>,
    // 'stream' borrows from 'request'
    stream: Option<IntoBytesStream<Pin<Box<dyn AsyncRead + Send>>>>,
    _pinned: std::marker::PhantomPinned,
}

pub enum PayloadKind {
    Empty,
    ReqRes(Pin<Box<ReqResPair>>),
}

impl ReqResPair {
    pub fn new(rocket: Arc<Rocket>) -> Pin<Box<ReqResPair>> {
        Box::pin(ReqResPair {
            rocket,
            request: None,
            response: None,
            stream: None,
            _pinned: std::marker::PhantomPinned,
        })
    }

    pub fn try_set_request<F, E>(self: Pin<&mut Self>, f: F) -> Result<(), E>
    where
        F: for<'r> FnOnce(&'r Rocket) -> Result<Request<'r>, E>
    {
        assert!(self.response.is_none(), "try_set_request was called after set_response");

        // Safety: 'rocket' is not &mut; 'request' is not yet considered pinned
        let (rocket, self_req_mut) = unsafe {
            let s = self.get_unchecked_mut();
            (&*s.rocket, &mut s.request)
        };

        f(rocket).map(|req| {
            // Safety: This structure keeps the Rocket instance (the data in 'r) alive longer than req.
            *self_req_mut = Some(unsafe { std::mem::transmute::<Request<'_>, Request<'static>>(req) });
        })
    }

    pub async fn set_response<F>(self: Pin<&mut Self>, f: F)
    where
        F: for<'a, 'r> FnOnce(&'a Rocket, &'r mut Request<'a>) -> BoxFuture<'r, Response<'r>>
    {
        assert!(self.request.is_some(), "set_response was called before try_set_request");
        // Setting a second response would require ensuring we drop the first one with its request 'live'
        assert!(self.response.is_none(), "set_response was called twice");

        // Safety: 'rocket' is not &mut; 'request' is not yet considered pinned;
        // 'response' is never considered pinned
        let (rocket, self_req_mut, self_res_mut) = unsafe {
            let s = self.get_unchecked_mut();
            (&*s.rocket, s.request.as_mut().unwrap(), &mut s.response)
        };

        // Safety: Shortening this lifetime is safe becuase Request is covariant over its lifetime parameter
        let req = unsafe { std::mem::transmute::<&mut Request<'static>, &mut Request<'_>>(self_req_mut) };
        let res = f(rocket, req).await;
        // Safety: This structure enforces the lifetime relationships
        *self_res_mut = Some(unsafe { std::mem::transmute::<Response<'_>, Response<'static>>(res) });
    }

    pub fn into_hyper_response(mut self: Pin<Box<Self>>) -> Result<hyper::Response<PayloadKind>, std::io::Error> {
        assert!(self.response.is_some(), "into_hyper_response was called before set_response");

        // Safety: 'response' is never considered pinned; 'stream' is never considered pinned
        let (response_mut, stream_mut) = unsafe {
            let s = self.as_mut().get_unchecked_mut();
            (s.response.as_mut().unwrap(), &mut s.stream)
        };

        let mut hyp_res = hyper::Response::builder();
        hyp_res = hyp_res.status(response_mut.status().code);

        for header in response_mut.headers().iter() {
            let name = header.name.as_str();
            let value = header.value.as_bytes();
            hyp_res = hyp_res.header(name, value);
        }

        let payload;

        match response_mut.take_body() {
            None => {
                hyp_res = hyp_res.header(header::CONTENT_LENGTH, "0");
                payload = PayloadKind::Empty;
            }
            Some(body) => {
                let (body, chunk_size) = match body {
                    Body::Chunked(body, chunk_size) => {
                        (body, chunk_size.try_into().expect("u64 -> usize overflow"))
                    }
                    Body::Sized(body, size) => {
                        hyp_res = hyp_res.header(header::CONTENT_LENGTH, size.to_string());
                        (body, 4096_usize)
                    }
                };

                // Safety: This structure keeps 'request' alive longer than 'stream'
                let fake_static_body: Pin<Box<dyn AsyncRead + Send + 'static>> = unsafe {
                    std::mem::transmute::<Pin<Box<dyn AsyncRead + Send + '_>>, Pin<Box<dyn AsyncRead + Send + 'static>>>(body)
                };
                *stream_mut = Some(fake_static_body.into_bytes_stream(chunk_size));
                payload = PayloadKind::ReqRes(self)
            }
        };

        hyp_res.body(payload).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

impl HttpBody for PayloadKind {
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

    fn poll_data(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        match *self {
            PayloadKind::Empty => Poll::Ready(None),
            PayloadKind::ReqRes(ref mut req_res) => {
                // Safety: PayloadKind can only be constructed by the same method that sets stream to Some
                let stream = unsafe { req_res.as_mut().map_unchecked_mut(|s| match s.stream.as_mut() { Some(s) => s, None => std::hint::unreachable_unchecked() }) };
                stream.poll_next(cx).map(|optres| optres.map(|res| res.map_err(|e| e.into())))
            }
        }
    }

    fn poll_trailers(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<Option<hyper::HeaderMap>, Self::Error>> {
        Poll::Ready(Ok(None))
    }
}

impl Drop for ReqResPair {
    fn drop(&mut self) {
        // Drop in the correct order
        self.stream = None;
        self.response = None;
        self.request = None;
    }
}
