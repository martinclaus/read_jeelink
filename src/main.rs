use serialport;
use std::{
    io::{ErrorKind, Read},
    time::Duration,
};

static DEVICE: &str = "/dev/tty.usbserial-AL006PX8";
const BAUT_RATE: u32 = 57600;
static TIMEOUT: Duration = Duration::from_secs(1);

fn main() -> std::io::Result<()> {
    println!("Open port on device");
    let mut port = serialport::new(DEVICE, BAUT_RATE)
        .timeout(TIMEOUT)
        .open_native()?;

    println!("Ready to read");
    let mut recorder = FrameRecorder::new();
    loop {
        let mut serial_buf: Vec<u8> = vec![0; 64];
        match port.read(serial_buf.as_mut_slice()) {
            Ok(n) => {
                serial_buf[..n]
                    .iter()
                    .filter_map(|&b| recorder.push(b as char))
                    .filter_map(|f| DecodedFrame::parse(f))
                    .for_each(|frame| println!("Got {:?}", frame));
            }
            Err(ref e) if e.kind() == ErrorKind::TimedOut => (),
            Err(e) => eprintln!("{:?}", e),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedFrame {
    id: u8,
    sensor_type: u8,
    new_battery: bool,
    weak_battery: bool,
    temperature: f32,
    humidity: u8,
}

impl DecodedFrame {
    fn parse(frame: Frame) -> Option<Self> {
        let fields: Vec<&str> = frame.0.split(' ').collect();

        let id: u8 = fields[0].parse().ok()?;

        let (new_battery, sensor_type) = {
            let field: u8 = fields[1].parse().ok()?;
            ((field / 128) != 0, field % 128)
        };

        let temp = {
            let field1: u16 = fields[2].parse().ok()?;
            let field2: u16 = fields[3].parse().ok()?;
            let temp: u16 = (field1 << 8) + field2;
            let temp: f32 = (temp as f32 - 1000.) / 10.;
            temp
        };

        let (weak_battery, hum) = {
            let field: u8 = fields[4].parse().ok()?;
            // first bit is weak battery flag
            ((field & 0x80 != 0), field & 0x7F)
        };

        Some(DecodedFrame {
            id,
            sensor_type,
            new_battery,
            weak_battery,
            temperature: temp,
            humidity: hum,
        })
    }
}

#[derive(Debug)]
struct Frame(String);

impl Frame {
    fn new(s: &str) -> Option<Frame> {
        if Frame::validate(s) {
            Some(Frame(s.to_string()))
        } else {
            None
        }
    }

    fn validate(s: &str) -> bool {
        {
            s.chars().all(|c| c.is_numeric() || c.is_whitespace())
                && s.chars().filter(|c| c.is_whitespace()).count() == 4
        }
    }
}
enum FrameRecorderState {
    NotRecording,
    Activating(usize),
    Recording,
    Terminating(usize),
}

impl FrameRecorderState {
    fn next(&mut self, len_activation: usize, len_termination: usize) {
        match self {
            FrameRecorderState::NotRecording => *self = FrameRecorderState::Activating(0),
            FrameRecorderState::Activating(level) => {
                *level += 1;
                if *level >= (len_activation - 1) {
                    *self = FrameRecorderState::Recording;
                }
            }
            FrameRecorderState::Recording => *self = FrameRecorderState::Terminating(0),
            FrameRecorderState::Terminating(level) => {
                *level += 1;
                if *level >= (len_termination - 1) {
                    *self = FrameRecorderState::NotRecording
                }
            }
        }
    }
}

pub struct FrameRecorder {
    buffer: String,
    state: FrameRecorderState,
    activate_chars: &'static [char],
    terminate_char: &'static [char],
}

impl FrameRecorder {
    pub fn new() -> Self {
        FrameRecorder {
            buffer: String::new(),
            state: FrameRecorderState::NotRecording,
            activate_chars: &['O', 'K', ' ', '9', ' '],
            terminate_char: &['\r', '\n'],
        }
    }

    fn push(&mut self, char: char) -> Option<Frame> {
        let n_act = self.activate_chars.len();
        let n_term = self.terminate_char.len();
        match self.state {
            FrameRecorderState::NotRecording => {
                if char == self.activate_chars[0] {
                    self.state.next(n_act, n_term);
                }
                None
            }
            FrameRecorderState::Activating(level) => {
                if char == self.activate_chars[level + 1] {
                    self.state.next(n_act, n_term)
                } else {
                    self.state = FrameRecorderState::NotRecording;
                }
                None
            }
            FrameRecorderState::Recording => {
                self.buffer.push(char);
                if char == self.terminate_char[0] {
                    self.state.next(n_act, n_term);
                }
                None
            }
            FrameRecorderState::Terminating(level) => {
                self.buffer.push(char);
                if char == self.terminate_char[level + 1] {
                    self.state.next(n_act, n_term)
                } else {
                    self.state = FrameRecorderState::Recording
                }
                match self.state {
                    FrameRecorderState::NotRecording => {
                        let frame = self.buffer.clone();
                        self.buffer.clear();
                        Frame::new(&frame[..frame.len() - 2])
                    }
                    _ => None,
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{DecodedFrame, Frame, FrameRecorder};

    #[test]
    fn test_frame_construction() {
        let data = [
            "OK 9 50 1 4 193 65\r\nOK 9 58 1 4 189 67\r\nOK 9 1 1 4 189 65\r\nOK 0 9 1",
            "OK 9 ",
            "\n[LaCrosseITPlusReader.10.1s (RFM69CW f:868300 t:30~3)",
            "]\r\n",
            "OK 9 13 1 4 181 ",
            "65\r\n",
            "OK 9 18 1 4 193 61\r\n",
            "OK 9 1 1 4 188 64\r\n",
        ];

        let mut recorder = FrameRecorder::new();

        let res: Vec<Frame> = data
            .iter()
            .flat_map(|s| s.chars())
            .filter_map(|c| recorder.push(c))
            .collect();

        let expect = [
            Frame("50 1 4 193 65".to_string()),
            Frame("58 1 4 189 67".to_string()),
            Frame("1 1 4 189 65".to_string()),
            Frame("13 1 4 181 65".to_string()),
            Frame("18 1 4 193 61".to_string()),
            Frame("1 1 4 188 64".to_string()),
        ];
        res.iter()
            .zip(expect.iter())
            .for_each(|(r, e)| assert_eq!(r.0, e.0));
    }
}
