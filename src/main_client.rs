use std::time::SystemTime;

use client::Client;
use hello_world::greeter_client::GreeterClient;
use wasm_rs_async_executor::single_threaded as executor;

use crate::hello_world::ProcessRequest;

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
    std::fs::write("/services/detector.special", "pls start :)")?;
    let start = SystemTime::now();
    let mut client = GreeterClient::new(Client::new("/services/detector.sock".into()));
    let req = tonic::Request::new(ProcessRequest {
        path: "/video_input/in.h264".to_string(),
    });
    let mut resp = client.process_video(req).await?;
    while let Ok(Some(msg)) = resp.get_mut().message().await {
        let elapsed = start.elapsed().unwrap();
        println!("[T+{}] {:?}", elapsed.as_secs(), msg);
    }
    println!("-");

    Ok(())
}
