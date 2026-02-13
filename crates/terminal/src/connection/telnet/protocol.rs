use alacritty_terminal::event::WindowSize;

// Telnet protocol command bytes (RFC 854)
const IAC: u8 = 255;  // Interpret As Command
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;   // Subnegotiation Begin
const SE: u8 = 240;   // Subnegotiation End

// Telnet option codes
const OPT_ECHO: u8 = 1;
const OPT_SUPPRESS_GO_AHEAD: u8 = 3;
const OPT_TERMINAL_TYPE: u8 = 24;
const OPT_NAWS: u8 = 31;  // Negotiate About Window Size

// Subnegotiation commands
const SB_IS: u8 = 0;
const SB_SEND: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserState {
    Data,
    Iac,
    Will,
    Wont,
    Do,
    Dont,
    Sb,
    SbData,
    SbIac,
}

pub struct TelnetNegotiator {
    state: ParserState,
    terminal_type: String,
    sb_option: u8,
    sb_data: Vec<u8>,
    naws_enabled: bool,
}

impl TelnetNegotiator {
    pub fn new(terminal_type: impl Into<String>) -> Self {
        Self {
            state: ParserState::Data,
            terminal_type: terminal_type.into(),
            sb_option: 0,
            sb_data: Vec::new(),
            naws_enabled: false,
        }
    }

    pub fn process_incoming(&mut self, data: &[u8]) -> ProcessResult {
        let mut output_data = Vec::new();
        let mut responses = Vec::new();

        for &byte in data {
            match self.state {
                ParserState::Data => {
                    if byte == IAC {
                        self.state = ParserState::Iac;
                    } else {
                        output_data.push(byte);
                    }
                }
                ParserState::Iac => {
                    match byte {
                        IAC => {
                            // Escaped IAC (255 255 → single 255)
                            output_data.push(IAC);
                            self.state = ParserState::Data;
                        }
                        WILL => self.state = ParserState::Will,
                        WONT => self.state = ParserState::Wont,
                        DO => self.state = ParserState::Do,
                        DONT => self.state = ParserState::Dont,
                        SB => self.state = ParserState::Sb,
                        SE => {
                            // Unexpected SE, ignore
                            self.state = ParserState::Data;
                        }
                        _ => {
                            // Unknown command, ignore
                            self.state = ParserState::Data;
                        }
                    }
                }
                ParserState::Will => {
                    responses.extend(self.handle_will(byte));
                    self.state = ParserState::Data;
                }
                ParserState::Wont => {
                    self.state = ParserState::Data;
                }
                ParserState::Do => {
                    responses.extend(self.handle_do(byte));
                    self.state = ParserState::Data;
                }
                ParserState::Dont => {
                    self.state = ParserState::Data;
                }
                ParserState::Sb => {
                    self.sb_option = byte;
                    self.sb_data.clear();
                    self.state = ParserState::SbData;
                }
                ParserState::SbData => {
                    if byte == IAC {
                        self.state = ParserState::SbIac;
                    } else {
                        self.sb_data.push(byte);
                    }
                }
                ParserState::SbIac => {
                    match byte {
                        SE => {
                            responses.extend(self.handle_subnegotiation());
                            self.state = ParserState::Data;
                        }
                        IAC => {
                            // Escaped IAC in subnegotiation
                            self.sb_data.push(IAC);
                            self.state = ParserState::SbData;
                        }
                        _ => {
                            // Protocol error, reset
                            self.state = ParserState::Data;
                        }
                    }
                }
            }
        }

        ProcessResult {
            data: output_data,
            responses,
        }
    }

    fn handle_will(&mut self, option: u8) -> Vec<u8> {
        match option {
            OPT_ECHO | OPT_SUPPRESS_GO_AHEAD => {
                // Accept these options from the server
                vec![IAC, DO, option]
            }
            _ => {
                // Refuse unknown options
                vec![IAC, DONT, option]
            }
        }
    }

    fn handle_do(&mut self, option: u8) -> Vec<u8> {
        match option {
            OPT_TERMINAL_TYPE => {
                // We will send terminal type
                vec![IAC, WILL, option]
            }
            OPT_NAWS => {
                self.naws_enabled = true;
                vec![IAC, WILL, option]
            }
            _ => {
                // Refuse unknown options
                vec![IAC, WONT, option]
            }
        }
    }

    fn handle_subnegotiation(&mut self) -> Vec<u8> {
        match self.sb_option {
            OPT_TERMINAL_TYPE => {
                if !self.sb_data.is_empty() && self.sb_data[0] == SB_SEND {
                    // Server wants our terminal type
                    let mut response = vec![IAC, SB, OPT_TERMINAL_TYPE, SB_IS];
                    response.extend(self.terminal_type.as_bytes());
                    response.extend([IAC, SE]);
                    return response;
                }
            }
            _ => {}
        }
        Vec::new()
    }

    pub fn build_naws(&self, size: WindowSize) -> Vec<u8> {
        if !self.naws_enabled {
            return Vec::new();
        }

        let cols = size.num_cols;
        let rows = size.num_lines;

        let mut packet = vec![IAC, SB, OPT_NAWS];

        // Width (2 bytes, big-endian) with IAC escaping
        let width_hi = (cols >> 8) as u8;
        let width_lo = (cols & 0xFF) as u8;
        packet.push(width_hi);
        if width_hi == IAC {
            packet.push(IAC);
        }
        packet.push(width_lo);
        if width_lo == IAC {
            packet.push(IAC);
        }

        // Height (2 bytes, big-endian) with IAC escaping
        let height_hi = (rows >> 8) as u8;
        let height_lo = (rows & 0xFF) as u8;
        packet.push(height_hi);
        if height_hi == IAC {
            packet.push(IAC);
        }
        packet.push(height_lo);
        if height_lo == IAC {
            packet.push(IAC);
        }

        packet.extend([IAC, SE]);
        packet
    }

    pub fn is_naws_enabled(&self) -> bool {
        self.naws_enabled
    }
}

pub struct ProcessResult {
    pub data: Vec<u8>,
    pub responses: Vec<u8>,
}

pub fn escape_data_for_send(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len());
    for &byte in data {
        result.push(byte);
        if byte == IAC {
            result.push(IAC);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_data_passes_through() {
        let mut negotiator = TelnetNegotiator::new("xterm-256color");
        let result = negotiator.process_incoming(b"hello world");
        assert_eq!(result.data, b"hello world");
        assert!(result.responses.is_empty());
    }

    #[test]
    fn test_iac_escape() {
        let mut negotiator = TelnetNegotiator::new("xterm-256color");
        let result = negotiator.process_incoming(&[b'a', IAC, IAC, b'b']);
        assert_eq!(result.data, &[b'a', IAC, b'b']);
    }

    #[test]
    fn test_will_echo_response() {
        let mut negotiator = TelnetNegotiator::new("xterm-256color");
        let result = negotiator.process_incoming(&[IAC, WILL, OPT_ECHO]);
        assert!(result.data.is_empty());
        assert_eq!(result.responses, &[IAC, DO, OPT_ECHO]);
    }

    #[test]
    fn test_do_terminal_type_response() {
        let mut negotiator = TelnetNegotiator::new("xterm-256color");
        let result = negotiator.process_incoming(&[IAC, DO, OPT_TERMINAL_TYPE]);
        assert!(result.data.is_empty());
        assert_eq!(result.responses, &[IAC, WILL, OPT_TERMINAL_TYPE]);
    }

    #[test]
    fn test_terminal_type_subnegotiation() {
        let mut negotiator = TelnetNegotiator::new("xterm-256color");
        // First enable terminal type
        let _ = negotiator.process_incoming(&[IAC, DO, OPT_TERMINAL_TYPE]);

        // Then request terminal type
        let result = negotiator.process_incoming(&[IAC, SB, OPT_TERMINAL_TYPE, SB_SEND, IAC, SE]);

        let mut expected = vec![IAC, SB, OPT_TERMINAL_TYPE, SB_IS];
        expected.extend(b"xterm-256color");
        expected.extend([IAC, SE]);

        assert_eq!(result.responses, expected);
    }

    #[test]
    fn test_naws_negotiation() {
        let mut negotiator = TelnetNegotiator::new("xterm-256color");

        // Server sends DO NAWS
        let result = negotiator.process_incoming(&[IAC, DO, OPT_NAWS]);
        assert_eq!(result.responses, &[IAC, WILL, OPT_NAWS]);
        assert!(negotiator.is_naws_enabled());

        // Build NAWS packet
        let size = WindowSize {
            num_cols: 80,
            num_lines: 24,
            cell_width: 8,
            cell_height: 16,
        };
        let naws = negotiator.build_naws(size);

        // Expected: IAC SB NAWS 0 80 0 24 IAC SE
        assert_eq!(naws, &[IAC, SB, OPT_NAWS, 0, 80, 0, 24, IAC, SE]);
    }

    #[test]
    fn test_escape_data_for_send() {
        let data = &[b'a', IAC, b'b'];
        let escaped = escape_data_for_send(data);
        assert_eq!(escaped, &[b'a', IAC, IAC, b'b']);
    }
}
