use std::hint::black_box;
use std::time::SystemTime;

use crate::hello_world::benchmark_client::BenchmarkClient;
use crate::hello_world::{Message, ThroughputRequest};
use client::Client;
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
            println!("Client err: {:?}", e);
        }
    });
    executor::run(None);
    Ok(())
}

async fn main_impl() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting..");
    std::fs::write("/services/server.special", "pls start :)")?;
    // let mut client = BenchmarkClient::new(Client::new("/services/server.sock".into()));
    // for size in [1usize, 16, 32, 64]
    //     .into_iter()
    //     .chain(
    //         (1..)
    //             .map(|x| (((x as f64).powf(1.5)) * 100.0) as usize)
    //             .take_while(|&x| x <= 100_000),
    //     )
    //     .skip_while(|&x| x < 36421)
    // {
    //     let mut bytes = Vec::new();
    //     bytes.resize(size, 'A' as u8);
    //     let msg = Message {
    //         bytes: String::from_utf8(bytes).unwrap(),
    //     };
    //     for _ in 0..5_000 {
    //         let req = tonic::Request::new(msg.clone());
    //         client.latency_echo(req).await?;
    //     }
    //     let start = SystemTime::now();
    //     for _ in 0..200_000 {
    //         let req = tonic::Request::new(msg.clone());
    //         client.latency_echo(req).await?;
    //     }
    //     let elapsed = start.elapsed().unwrap();
    //     println!("{},{}", size, elapsed.as_micros() / 200_000);
    // }
    let mut client = BenchmarkClient::new(Client::new("/services/server.sock".into()));
    for buffer_size in [32678]
    {
        let msg = ThroughputRequest { buffer_size };
        let req = tonic::Request::new(msg.clone());
        let start = SystemTime::now();
        let mut resp = client.throughput_test(req).await?.into_inner();
        while let Some(_) = resp.message().await? {
        }
        let elapsed = start.elapsed().unwrap();

        println!("{},{}", buffer_size, elapsed.as_micros());
    }

    Ok(())
}
