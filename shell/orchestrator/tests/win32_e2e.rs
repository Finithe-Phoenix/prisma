use prisma_orchestrator::win32::gdi32;
use prisma_orchestrator::win32::kernel32;
use prisma_orchestrator::win32::user32;

#[test]
fn test_win32_synthetic_modules() {
    // Call mock functions from kernel32
    kernel32::LoadLibraryA();
    kernel32::CreateFileW();

    // Call mock functions from user32
    user32::RegisterClassExW();
    user32::CreateWindowExW();
    user32::ShowWindow();
    user32::UpdateWindow();
    user32::GetMessageW();
    user32::TranslateMessage();
    user32::DispatchMessageW();
    user32::DefWindowProcW();

    // Call mock functions from gdi32
    gdi32::BeginPaint();
    gdi32::EndPaint();
    gdi32::GetDC();
    gdi32::ReleaseDC();
    gdi32::TextOutW();
    gdi32::FillRect();
    gdi32::CreateSolidBrush();

    // If it reaches here without panicking, the test passes
}
