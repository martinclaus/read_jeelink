use serialport::{self, SerialPort};
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{ErrorKind, Read},
    str::FromStr,
    time::Duration,
};

static DEVICE: &str = "/dev/tty.usbserial-AL006PX8";
const BAUT_RATE: u32 = 57600;
static TIMEOUT: Duration = Duration::from_secs(1);

fn main() -> std::io::Result<()> {
    println!("Open port on device");
    let listener = SerialListener::bind(DEVICE)?;
    println!("Ready to read");
    for frame in listener.incomming() {
        let frame = frame?;
        println!("Got {:?}", frame);
    }
    Ok(())
}

pub struct SerialListener {
    port: RefCell<Box<dyn SerialPort>>,
    recorder: RefCell<FrameRecorder>,
}

impl SerialListener {
    pub fn bind(addr: &str) -> Result<SerialListener, std::io::Error> {
        let port = serialport::new(addr, BAUT_RATE).timeout(TIMEOUT).open()?;
        let recorder = FrameRecorder::new();
        Ok(SerialListener {
            port: RefCell::new(port),
            recorder: RefCell::new(recorder),
        })
    }

    /// Blocks until at least one complete frame arrived.
    pub fn accept(&self) -> std::io::Result<Vec<Frame>> {
        let mut frames: Vec<Frame> = vec![];
        let mut read_buf = [0u8; 1024];
        let mut port = self.port.borrow_mut();
        let mut recorder = self.recorder.borrow_mut();
        while frames.is_empty() {
            match port.read(&mut read_buf) {
                // read n bytes
                Ok(n) => {
                    frames.extend(
                        read_buf[..n]
                            .iter()
                            .filter_map(|&b| recorder.push(b as char))
                            .filter_map(|s| s.parse::<Frame>().ok())
                            .collect::<Vec<Frame>>(),
                    );
                }
                // no data received, keep trying
                Err(ref e) if e.kind() == ErrorKind::TimedOut => (),
                // Some other error happened
                Err(e) => eprintln!("{:?}", e),
            }
        }
        Ok(frames)
    }

    /// Return an iterator that accepts indefinately incomming frames.
    pub fn incomming(&self) -> Incoming {
        Incoming {
            listener: self,
            frame_buffer: VecDeque::new(),
        }
    }
}

pub struct Incoming<'a> {
    listener: &'a SerialListener,
    frame_buffer: VecDeque<Frame>,
}

impl<'a> Iterator for Incoming<'a> {
    type Item = std::io::Result<Frame>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.frame_buffer.is_empty() {
            match self.listener.accept() {
                Ok(frames) => self.frame_buffer.extend(frames),
                Err(e) => return Some(Err(e)),
            }
        }
        Some(Ok(self
            .frame_buffer
            .pop_front()
            .expect("Framebuffer empty")))
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    id: u8,
    sensor_type: u8,
    new_battery: bool,
    weak_battery: bool,
    temperature: f32,
    humidity: u8,
}

impl Frame {
    fn parse(s: &str) -> Option<Self> {
        let fields: Vec<&str> = s.split(' ').collect();

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

        Some(Frame {
            id,
            sensor_type,
            new_battery,
            weak_battery,
            temperature: temp,
            humidity: hum,
        })
    }

    fn validate(s: &str) -> bool {
        {
            s.chars().all(|c| c.is_numeric() || c.is_whitespace())
                && s.chars().filter(|c| c.is_whitespace()).count() == 4
        }
    }
}

impl FromStr for Frame {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if Self::validate(s) {
            Self::parse(s).ok_or("Cannot parse message")
        } else {
            Err("Not a valid message")
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

    fn push(&mut self, char: char) -> Option<String> {
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
                        Some(frame[..frame.len() - 2].to_string())
                    }
                    _ => None,
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::FrameRecorder;

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

        let res: Vec<String> = data
            .iter()
            .flat_map(|s| s.chars())
            .filter_map(|c| recorder.push(c))
            .collect();

        let expect = [
            "50 1 4 193 65",
            "58 1 4 189 67",
            "1 1 4 189 65",
            "13 1 4 181 65",
            "18 1 4 193 61",
            "1 1 4 188 64",
        ];
        res.into_iter()
            .zip(expect.into_iter())
            .for_each(|(r, e)| assert_eq!(r, e));
    }
}
