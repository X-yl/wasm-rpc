use std::sync::atomic::AtomicBool;

use crate::hello_world::benchmark_client::BenchmarkClient;
use crate::hello_world::Message;
use async_trait::async_trait;
use hello_world::{
    benchmark_server::{Benchmark, BenchmarkServer},
    ThroughputRequest,
};
use server::ReceiverStream;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::broadcast;
use tonic::{Request, Response, Status};

use crate::server::BroadcastReceiverStream;
use crate::server::Server;

#[allow(non_snake_case)]
pub mod hello_world {
    tonic::include_proto!("helloworld");
}
mod http2;
mod server;

pub struct Bencher {
    tx: Option<broadcast::Sender<Result<Message, Status>>>,
    rx: Option<broadcast::Receiver<Result<Message, Status>>>,
    got_subscriber: &'static AtomicBool
}

#[async_trait]
impl Benchmark for Bencher {
    async fn latency_echo(
        &self,
        request: tonic::Request<Message>,
    ) -> Result<tonic::Response<Message>, Status> {
        Ok(tonic::Response::new(request.into_inner()))
    }

    type ThroughputTestStream = ReceiverStream<Result<Message, Status>>;
    type MultiClientThroughputRxStream = ReceiverStream<Result<Message, Status>>;

    async fn throughput_test(
        &self,
        request: tonic::Request<ThroughputRequest>,
    ) -> Result<Response<Self::ThroughputTestStream>, Status> {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let buffer_size = request.get_ref().buffer_size as usize;
            bytes.resize(buffer_size, 'A' as u8);
            let s = String::from_utf8(bytes).unwrap();
            for _ in 0..(10_000_000_000 / buffer_size) {
                tx.send(Ok(Message { bytes: s.clone() })).await.unwrap();
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn multi_client_throughput_rx(
        &self,
        request: tonic::Request<ThroughputRequest>,
    ) -> Result<Response<Self::MultiClientThroughputRxStream>, Status> {
        println!("got a subscriber!");
        let mut rx_clone = self.rx.as_ref().unwrap().resubscribe();
        let (tx2, rx2) = mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                tx2.send(rx_clone.recv().await.unwrap()).await.unwrap();
            }
        });
        Ok(Response::new(ReceiverStream::new(rx2)))
    }

    async fn multi_client_throughput_tx(
        &self,
        request: Request<tonic::Streaming<Message>> 
    ) -> Result<Response<Message>, Status> {
        let tx_clone = self.tx.clone().unwrap();
        tokio::spawn(async move {
            let mut stream = request.into_inner();
            while let Some(msg) = stream.message().await.unwrap() {
                tx_clone.send(Ok(msg)).unwrap();
            }
        });
        Ok(Response::new(Message { bytes: "ok".to_string() }))
    }

}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = broadcast::channel(32);
    let got_subscriber = Box::leak(Box::new(false.into()));
    let bencher = Bencher {
        tx: Some(tx),
        rx: None,
        got_subscriber
    };

    let bencher2 = Bencher {
        tx: None,
        rx: Some(rx),
        got_subscriber
    };

    let bencher_server = BenchmarkServer::new(bencher);
    let server = Server::new("/services/server.sock".into(), bencher_server);
    println!("Serving on server.sock");

    let bencher_server2 = BenchmarkServer::new(bencher2);
    let server2 = Server::new("/services/server2.sock".into(), bencher_server2);
    println!("Serving on server2.sock");
    tokio::try_join!(server.serve(), server2.serve())?;

    Ok(())
}

// #[cfg(unix)]
// #[tokio::main]
// async fn main() -> Result<(), Box<dyn std::error::Error>> {
//     let greeter = Bencher;

//     let uds = tokio::net::UnixListener::bind("/services/server.sock")?;
//     let uds_stream = tokio_stream::wrappers::UnixListenerStream::new(uds);

//     println!("Server binding :)");

//     tonic::transport::Server::builder()
//         .add_service(BenchmarkServer::new(greeter))
//         .serve_with_incoming(uds_stream)
//         .await?;

//     Ok(())
// }