use crate::hello_world::benchmark_client::BenchmarkClient;
use crate::hello_world::Message;
use async_trait::async_trait;
use hello_world::{
    benchmark_server::{Benchmark, BenchmarkServer},
    ThroughputRequest,
};
use server::ReceiverStream;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tonic::{Request, Response, Status};

use crate::server::Server;

#[allow(non_snake_case)]
pub mod hello_world {
    tonic::include_proto!("helloworld");
}
mod http2;
mod server;

pub struct Bencher;

#[async_trait]
impl Benchmark for Bencher {
    async fn latency_echo(
        &self,
        request: tonic::Request<Message>,
    ) -> Result<tonic::Response<Message>, Status> {
        Ok(tonic::Response::new(request.into_inner()))
    }

    type ThroughputTestStream = ReceiverStream<Result<Message, Status>>;

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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bencher = Bencher {};
    let bencher_server = BenchmarkServer::new(bencher);
    let server = Server::new("/services/server.sock".into(), bencher_server);
    server.serve().await?;

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