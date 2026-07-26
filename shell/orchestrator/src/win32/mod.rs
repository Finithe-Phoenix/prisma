pub mod kernel32;
pub mod ntdll;
pub mod user32;
pub mod gdi32;

pub struct Win32Environment {}

impl Win32Environment {
    pub fn new() -> Self { Self {} }
}
