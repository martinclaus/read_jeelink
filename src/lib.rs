extern crate anyhow;
use std::time::Duration;

/// Baud rate of the device. For the JeeLink it is 57.6 KBd
pub const BAUD_RATE: u32 = 57600;

/// How long to listen before a time out error is issued.
/// This number does not have a sinificant meaning except that a low timeout may cause high CPU usage.
pub static TIMEOUT: Duration = Duration::from_millis(1000);

// Rexport main API
pub use frame::error::*;
pub use frame::Frame;
pub use listener::error::*;
pub use listener::SerialPortListener;

/// Listen on data frames from the Jeelink device
pub mod listener {
    use crate::frame::{error::FrameCheck::Incomplete, Frame};
    use bytes::{Buf, BytesMut};

    /// Listener on serial port
    ///
    /// Allows to read frames from device stream.
    pub struct SerialPortListener<P> {
        port: P,
        buffer: BytesMut,
    }

    impl<P> SerialPortListener<P> {
        pub fn new(port: P) -> SerialPortListener<P> {
            SerialPortListener {
                port,
                // Allocate buffer with 256 bytes
                buffer: BytesMut::with_capacity(256),
            }
        }

        fn parse(&mut self) -> anyhow::Result<Option<Frame>> {
            match Frame::check(&mut self.buffer) {
                Ok(frame_data) => {
                    // parse frame
                    let frame = Frame::parse(std::str::from_utf8(frame_data.chunk())?)?;
                    Ok(Some(frame))
                }
                Err(Incomplete) => Ok(None),
                Err(err) => Err(err.into()),
            }
        }
    }

    /// Read data frames from Jeelink asynchroneously.
    mod asynchroneous {
        use super::{error, SerialPortListener};
        use crate::frame::Frame;
        use tokio::io::AsyncReadExt;

        impl SerialPortListener<tokio_serial::SerialStream> {
            pub async fn read_frame(&mut self) -> anyhow::Result<Option<Frame>> {
                loop {
                    if let Some(frame) = self.parse()? {
                        return Ok(Some(frame));
                    }

                    if 0 == self.port.read_buf(&mut self.buffer).await? {
                        // stream closed. If buffer empty, normal close.
                        if self.buffer.is_empty() {
                            return Ok(None);
                        } else {
                            return Err(error::ListenerError::ConnectionLost)?;
                        }
                    }
                }
            }
        }
    }

    /// Synchroneous listener.
    ///
    /// This implementation arose in the learing process and is left here for testing purposes.
    pub mod synchroneous {
        use super::SerialPortListener;
        use crate::frame::Frame;
        use serialport::TTYPort;
        use std::io::Read;

        impl SerialPortListener<TTYPort> {
            pub fn read_frame(&mut self) -> anyhow::Result<Option<Frame>> {
                let mut stack_buf = [b'0'; 256];
                loop {
                    if let Some(frame) = self.parse()? {
                        return Ok(Some(frame));
                    }

                    match self.port.read(&mut stack_buf) {
                        Ok(n) if n == 0 => (),
                        Ok(n) => self.buffer.extend_from_slice(&stack_buf[0..n]),
                        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => (),
                        Err(e) => return Err(e)?,
                    }
                }
            }
        }
    }

    pub mod error {
        use thiserror::Error;

        #[derive(Error, Debug, PartialEq)]
        pub enum ListenerError {
            #[error("Connection lost to serial device")]
            ConnectionLost,
        }
    }
}

/// Module for creating data frames from the char stream read from the JeeLink LaCrosse firmware by FHEM.
mod frame {
    use core::fmt;
    use std::{fmt::Display, str::FromStr};

    use bytes::{Buf, BytesMut};

    /// Data Frame received from JeeLink device
    #[derive(Debug, Clone, PartialEq)]
    pub struct Frame {
        pub id: u8,
        pub sensor_type: u8,
        pub new_battery: bool,
        pub weak_battery: bool,
        pub temperature: f32,
        pub humidity: u8,
    }

    impl Frame {
        /// Check if a full frame is available in the buffer and returns it if possible.
        ///
        /// The input buffer will be advanced until a start sequence of a frame is reached.
        /// If a complete frame is in the buffer, the frames payload will be extraced and returned, and
        /// the frame data will be remove from the buffer.
        /// If no complete frame is found, the error FrameCheck::Incomplete is returned.
        pub fn check(buf: &mut BytesMut) -> Result<BytesMut, error::FrameCheck> {
            const START_SEQ: &[u8; 5] = b"OK 9 ";
            const END_SEQ: &[u8; 2] = b"\r\n";
            loop {
                if buf.remaining() <= START_SEQ.len() {
                    return Err(error::FrameCheck::Incomplete);
                }
                if buf.chunk().starts_with(START_SEQ) {
                    break;
                }
                buf.advance(1)
            }

            if let Some((i, _)) = buf.windows(2).enumerate().find(|(_, win)| win == END_SEQ) {
                let mut frame_data = buf.split_to(i);
                frame_data.advance(START_SEQ.len());
                buf.advance(END_SEQ.len());
                Ok(frame_data)
            } else {
                Err(error::FrameCheck::Incomplete)
            }
        }

        /// Convert a string to a Frame object. The string must be validated before parsing.
        pub fn parse(s: &str) -> anyhow::Result<Self> {
            Self::validate(s)?;

            let fields: Vec<&str> = s.split(|b| b == ' ').collect();

            let id: u8 = fields[0].parse()?;

            let (new_battery, sensor_type) = {
                let field: u8 = fields[1].parse()?;
                ((field / 128) != 0, field % 128)
            };

            let temp = {
                let field1: u16 = fields[2].parse()?;
                let field2: u16 = fields[3].parse()?;
                let temp: u16 = (field1 << 8) + field2;
                let temp: f32 = (temp as f32 - 1000.) / 10.;
                temp
            };

            let (weak_battery, hum) = {
                let field: u8 = fields[4].parse()?;
                // first bit is weak battery flag
                ((field & 0x80 != 0), field & 0x7F)
            };

            Ok(Frame {
                id,
                sensor_type,
                new_battery,
                weak_battery,
                temperature: temp,
                humidity: hum,
            })
        }

        /// Validate string to be parsable as a Frame object.
        fn validate(s: &str) -> Result<(), error::FrameValidation> {
            if !s.chars().all(|c| {
                c.is_numeric() || c.is_whitespace() || c.is_ascii_control() || c.is_control()
            }) {
                return Err(error::FrameValidation::InvalidChars(s.to_string()));
            }
            if s.chars().filter(|c| c.is_whitespace()).count() != 4 {
                return Err(error::FrameValidation::WrongNumberOfFields(s.to_string()));
            }
            Ok(())
        }
    }

    impl FromStr for Frame {
        type Err = anyhow::Error;

        /// Enables to use of str::parse to create a Frame object from a string
        ///
        /// # Example
        /// ```rust
        /// use read_jeelink::Frame;
        /// let parsed_frame: Frame = "50 1 4 193 65".parse().unwrap();
        /// ```
        fn from_str(s: &str) -> anyhow::Result<Self> {
            Self::parse(s)
        }
    }

    impl Display for Frame {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "Sensor {:2}: Temperatur {:4}, Humidity {:2}, weak battery: {}, new battery: {}",
                self.id, self.temperature, self.humidity, self.weak_battery, self.new_battery
            )
        }
    }

    pub mod error {
        use thiserror::Error;

        #[derive(Error, Debug, PartialEq)]
        pub enum FrameCheck {
            #[error("No complete frame in buffer")]
            Incomplete,
            #[error("Other error occured: {0}")]
            Other(String),
        }

        #[derive(Error, Debug, PartialEq)]
        pub enum FrameValidation {
            #[error("Frame data contains invalid characters. Allowed characters are numeric and whitespace; got: {0}")]
            InvalidChars(String),
            #[error("Frame requires 5 fields separated by whitespace. Got {0}")]
            WrongNumberOfFields(String),
        }
    }

    #[cfg(test)]
    mod test {
        use super::{error::FrameCheck, Frame};
        use bytes::BytesMut;

        #[test]
        fn test_frame_parsing() {
            let frame: Frame = "50 1 4 193 65".parse().unwrap();
            assert_eq!(
                frame,
                Frame {
                    id: 50,
                    sensor_type: 1,
                    new_battery: false,
                    weak_battery: false,
                    temperature: 21.7,
                    humidity: 65
                }
            );
        }

        #[test]
        fn test_frame_check_detects_incomplete_frame() {
            assert_eq!(
                Frame::check(&mut BytesMut::from(&b"OK 9 93 954 29"[..])),
                Err(FrameCheck::Incomplete)
            );
        }

        #[test]
        fn test_frame_check_extracts_frame_data() {
            assert_eq!(
                Frame::check(&mut BytesMut::from(
                    &b"45 2 5OK 9 93 954 29\r\nOK 9 25 24 63\r\n"[..]
                )),
                Ok(BytesMut::from(&b"93 954 29"[..]))
            )
        }

        #[test]
        fn test_frame_check_drops_frame_from_read_buffer() {
            let mut buf = BytesMut::from(&b"45 2 5OK 9 93 954 29\r\nOK 9 25 24 63\r\n"[..]);
            let _ = Frame::check(&mut buf);
            assert_eq!(buf, &b"OK 9 25 24 63\r\n"[..]);
        }
    }
}
