# gRPC and Streaming Technical Overview

There are a total of three main components:

- Changes to Veracruz (https://github.com/X-yl/veracruz/tree/grpc)
- A library which implements the custom transport for gRPC (https://github.com/X-yl/wasm-rpc)
- Changes to VOD for the streaming demo (https://github.com/X-yl/video-object-detection/tree/dynamic)

## Changes to Veracruz

These involve:

- Modifying VFS to add a Socket file type
- Modifying native-module-manager to detatch native modules
- Modifying the core architecture to add the second stream between the enclave and vc-server
- Modifying the VFS so writes to stdout are forwarded to vc-server
- Modifying the CLI for vc-client so a second client is spawned to connect to the output stream

No building changes.

## Library which implements custom transport for gRPC

- client.rs is a candidate for inclusion in libveracruz. It provides the transport mechanism
- main\_client.rs is the driver for the client and demonstrates tonic. It contains the benchmarking code.
- main\_server.rs is the driver for the server and demonstrates tonic. It contains benchmarking code.
- server.rs is legacy and not required. It implements the HTTP/2 server for the native side, but this is redundant
  as tonic's built in transport can be used.

### Building

For the client:

```
cargo build --bin client --target wasm32-wasi
```

For the server:

```
cargo build --bin server --target x86_64-unknown-linux-gnu
```

## Streaming VOD

This involves adding the gRPC C++ library and modifying the C++ code to use it.

## Building

Follow the instructions in the README.md to set up VOD.
Then install gRPC as per the [gRPC quickstart](https://grpc.io/docs/languages/cpp/quickstart/#install-grpc).

Assuming your install path is $GRPC\_PATH, build the detector binary with:

```
GRPC_PATH=$GRPC_PATH make -f Makefile_native
```

Then, to run the demo, use demo.py:

```
VERACRUZ_PATH=/work/veracruz python demo.py
```

