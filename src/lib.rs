use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Instant, Duration};
use tokio_trace_core::{field, Event, Metadata, Span};

enum Message {
    Done,
    Event(Value),
}

pub struct MaybeChromeTraceSubscriber(pub Option<ChromeTraceSubscriber>);

pub struct ChromeTraceSubscriber {
    start: Instant,
    next_span: Arc<AtomicUsize>,
    tx: Arc<Mutex<Sender<Message>>>,
}

impl ChromeTraceSubscriber {
    pub fn new(writer: File) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            writer_thread(rx, writer)
        });
        ChromeTraceSubscriber {
            start: Instant::now(),
            next_span: Arc::new(AtomicUsize::new(0)),
            tx: Arc::new(Mutex::new(tx)),
        }
    }
}

fn writer_thread(rx: Receiver<Message>, mut writer: File) {
    drop(writeln!(writer, "["));
    while let Ok(msg) = rx.recv() {
        match msg {
            Message::Done => {
                drop(writeln!(writer, "]"));
                break;
            }
            Message::Event(val) => {
                drop(serde_json::to_writer(&writer, &val));
                // Add a trailing comma because we're writing a JSON array.
                drop(writeln!(writer, ","));
            }
        }
    }
}

impl Drop for ChromeTraceSubscriber {
    fn drop(&mut self) {
        drop(self.tx.lock().unwrap().send(Message::Done))
    }
}

impl tokio_trace_core::Subscriber for ChromeTraceSubscriber {
    fn enabled(&self, _metadata: &Metadata) -> bool { true }

    fn new_span(&self, _metadata: &Metadata, _values: &field::ValueSet) -> Span {
        Span::from_u64(self.next_span.fetch_add(10, Ordering::SeqCst) as u64)
    }

    fn record(&self, _span: &Span, _values: &field::ValueSet) {}

    fn record_follows_from(&self, _span: &Span, _follows: &Span) {}

    fn event(&self, event: &Event) {
        let ts = self.start.elapsed();
        let meta = event.metadata();
        let mut rec = Recorder::new();
        event.record(&mut rec);
        let Recorder { message, fields } = rec;
        let val = json!({
            "name": message.unwrap_or("<unknown>".to_owned()),
            "cat": meta.target(),
            "ph": "I",
            "ts": in_micros(ts),
            "s": "p",
            "pid": process::id(),
            "tid": thread_id::get(),
            "args": fields,
        });
        drop(self.tx.lock().unwrap().send(Message::Event(val)))
    }

    fn enter(&self, _span: &Span) {}

    fn exit(&self, _span: &Span) {}
}

impl tokio_trace_core::Subscriber for MaybeChromeTraceSubscriber {
    fn enabled(&self, _metadata: &Metadata) -> bool { self.0.is_some() }

    fn new_span(&self, metadata: &Metadata, values: &field::ValueSet) -> Span {
        match self.0 {
            Some(ref s) => s.new_span(metadata, values),
            None => Span::from_u64(0),
        }
    }

    fn record(&self, span: &Span, values: &field::ValueSet) {
        match self.0 {
            Some(ref s) => s.record(span, values),
            None => {}
        }
    }

    fn record_follows_from(&self, span: &Span, follows: &Span) {
        match self.0 {
            Some(ref s) => s.record_follows_from(span, follows),
            None => {}
        }
    }

    fn event(&self, event: &Event) {
        match self.0 {
            Some(ref s) => s.event(event),
            None => {}
        }
    }

    fn enter(&self, span: &Span) {
        match self.0 {
            Some(ref s) => s.enter(span),
            None => {}
        }
    }

    fn exit(&self, span: &Span) {
        match self.0 {
            Some(ref s) => s.exit(span),
            None => {}
        }
    }
}

fn in_micros(d: Duration) -> u64 {
    1000000 * d.as_secs() + (d.subsec_nanos() / 1000) as u64
}

struct Recorder {
    pub message: Option<String>,
    pub fields: HashMap<&'static str, String>,
}

impl Recorder {
    pub fn new() -> Self {
        Recorder {
            message: None,
            fields: HashMap::new(),
        }
    }
}


impl field::Record for Recorder {
    fn record_str(&mut self, field: &field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.fields.insert(field.name(), value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &field::Field, value: &fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        } else {
            self.fields.insert(field.name(), format!("{:?}", value));
        }
    }
}
