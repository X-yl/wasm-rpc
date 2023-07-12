use http2parse::*;

pub const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub fn prepare_initial_settings() -> [Frame<'static>; 2] {
    let payload1 = Payload::Settings(&[]);
    let settings = Frame {
        header: FrameHeader {
            length: payload1.encoded_len() as u32,
            kind: Kind::Settings,
            flag: Flag::empty(),
            id: http2parse::StreamIdentifier(0),
        },
        payload: payload1,
    };

    let payload2 = Payload::WindowUpdate(http2parse::SizeIncrement(2u32.pow(31) - 1));
    let window_update = Frame {
        header: FrameHeader {
            length: payload2.encoded_len() as u32,
            kind: Kind::WindowUpdate,
            flag: Flag::empty(),
            id: http2parse::StreamIdentifier(0),
        },
        payload: payload2,
    };

    [settings, window_update]
}
