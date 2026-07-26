pub mod kernel32;
pub mod ntdll;

pub struct Win32Environment {}

impl Win32Environment {
    pub fn new() -> Self { Self {} }
}
