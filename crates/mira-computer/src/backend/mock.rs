//! In-memory backend for tests and evals: records every input event and
//! serves a solid-color "screen".

use std::sync::Mutex;

use async_trait::async_trait;
use image::RgbaImage;

use super::{ButtonAction, ComputerBackend, KeyAction};
use crate::action::{MouseButton, Point, ScrollDirection};
use crate::keys::KeyChord;
use crate::ComputerError;

/// One recorded input event, in input space.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Move(Point),
    Button(MouseButton, ButtonAction),
    Scroll(ScrollDirection, u32),
    Type(String),
    Key(KeyChord, KeyAction),
}

pub struct MockBackend {
    /// Input-space size.
    pub screen: (u32, u32),
    /// Capture resolution (e.g. 2× `screen` to simulate Retina).
    pub capture_size: (u32, u32),
    pub events: Mutex<Vec<Event>>,
    cursor: Mutex<Point>,
}

impl MockBackend {
    pub fn new(screen: (u32, u32), capture_size: (u32, u32)) -> Self {
        Self {
            screen,
            capture_size,
            events: Mutex::new(Vec::new()),
            cursor: Mutex::new(Point { x: 0, y: 0 }),
        }
    }

    pub fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    fn push(&self, e: Event) {
        self.events.lock().unwrap().push(e);
    }
}

#[async_trait]
impl ComputerBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn screen_size(&self) -> Result<(u32, u32), ComputerError> {
        Ok(self.screen)
    }

    async fn capture(&self) -> Result<RgbaImage, ComputerError> {
        Ok(RgbaImage::from_pixel(
            self.capture_size.0,
            self.capture_size.1,
            image::Rgba([30, 60, 90, 255]),
        ))
    }

    async fn mouse_move(&self, p: Point) -> Result<(), ComputerError> {
        *self.cursor.lock().unwrap() = p;
        self.push(Event::Move(p));
        Ok(())
    }

    async fn button(&self, button: MouseButton, action: ButtonAction) -> Result<(), ComputerError> {
        self.push(Event::Button(button, action));
        Ok(())
    }

    async fn scroll(&self, direction: ScrollDirection, amount: u32) -> Result<(), ComputerError> {
        self.push(Event::Scroll(direction, amount));
        Ok(())
    }

    async fn type_text(&self, text: &str) -> Result<(), ComputerError> {
        self.push(Event::Type(text.to_owned()));
        Ok(())
    }

    async fn key(&self, chord: &KeyChord, action: KeyAction) -> Result<(), ComputerError> {
        self.push(Event::Key(chord.clone(), action));
        Ok(())
    }

    async fn cursor_position(&self) -> Result<Point, ComputerError> {
        Ok(*self.cursor.lock().unwrap())
    }
}
