/// The most a message may grow before the stream is dropped as broken.
const MAX_MESSAGE: usize = 1_000_000;

/// Splits a server-sent event stream into messages: the text between blank
/// lines. Feed it chunks as they arrive.
#[derive(Default)]
pub struct Splitter {
    buffer: String,
}

impl Splitter {
    /// The messages completed by `chunk`, or `None` once a message is too big.
    pub fn push(&mut self, chunk: &[u8]) -> Option<Vec<String>> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        if self.buffer.len() > MAX_MESSAGE {
            return None;
        }
        if self.buffer.contains('\r') {
            self.buffer = self.buffer.replace("\r\n", "\n");
        }
        let mut messages = Vec::new();
        while let Some(end) = self.buffer.find("\n\n") {
            let message: String = self.buffer.drain(..end + 2).collect();
            messages.push(message.trim_end().to_owned());
        }
        Some(messages)
    }
}

/// The value of a field such as `id:` in one message.
pub fn field<'a>(message: &'a str, name: &str) -> Option<&'a str> {
    message
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':').map(str::trim))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_messages_across_chunks() {
        let mut splitter = Splitter::default();
        assert_eq!(
            splitter.push(b"id: 1\ndata: {}\n").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            splitter.push(b"\nid: 2\r\ndata: {}\r\n\r\n").unwrap(),
            vec!["id: 1\ndata: {}", "id: 2\ndata: {}"]
        );
    }

    #[test]
    fn reads_a_field() {
        assert_eq!(field("id: 42\ndata: {}", "id"), Some("42"));
        assert_eq!(field("data: {}", "id"), None);
    }
}
