use std::borrow::BorrowMut;
use std::fmt::format;
use std::hint::black_box;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll};
use std::time::SystemTime;

use client::Client;
use hello_world::ProcessRequest;
use hello_world::greeter_client::GreeterClient;
use tonic::Request;
use tonic::codegen::futures_core::Stream;
use wasm_rs_async_executor::single_threaded as executor;

#[allow(non_snake_case)]
pub mod hello_world {
    tonic::include_proto!("helloworld");
}

mod client;
mod http2;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    executor::spawn(async move {
        if let Err(e) = main_impl().await {
            println!("2Client err: {:?}", e);
        }
    });
    executor::run(None);
    Ok(())
}

async fn main_impl() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write("/services/detector.special", "pls start :)").unwrap();
    let mut client = GreeterClient::new(Client::new("/services/detector.sock".into()));

    let message = ProcessRequest {
        path: "/video_input/in.h264".to_string()
    };

    println!("Processing video");
    let mut resp = client.process_video(Request::new(message)).await.unwrap().into_inner();

    while let Some(status) = resp.message().await.unwrap() {
        println!("frame-status|{}|{:?}",status.frame_count, status.items.iter().map(|it| {
            format!("{},{},{},{},{},{}", it.object, it.probability, it.x, it.y, it.w, it.h)
        }).collect::<Vec<_>>())        
    }


    Ok(())
}

pub struct IterStream<T, It>
where
    It: Iterator<Item = T>,
{
    inner: It,
}

impl<T, It> IterStream<T, It>
where
    It: Iterator<Item = T>,
{
    /// Create a new `ReceiverStream`.
    pub fn new(recv: It) -> Self {
        Self { inner: recv }
    }
}
impl<T, It> Unpin for IterStream<T, It> where It: Iterator<Item = T> {}

impl<T, It> Stream for IterStream<T, It>
where
    It: Iterator<Item = T>,
{
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.inner.next())
    }
}
