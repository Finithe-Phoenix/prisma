#include <windows.h>

LRESULT CALLBACK WindowProc(HWND hwnd, UINT uMsg, WPARAM wParam, LPARAM lParam) {
  switch (uMsg) {
    case WM_PAINT: {
      PAINTSTRUCT ps;
      HDC hdc = BeginPaint(hwnd, &ps);

      HBRUSH brush = CreateSolidBrush(RGB(0, 0, 255));
      FillRect(hdc, &ps.rcPaint, brush);

      TextOutW(hdc, 50, 50, L"Hello Prisma XP!", 16);

      EndPaint(hwnd, &ps);
      return 0;
    }
    case WM_DESTROY:
      PostQuitMessage(0);
      return 0;
  }
  return DefWindowProcW(hwnd, uMsg, wParam, lParam);
}

int WINAPI WinMain(HINSTANCE hInstance, HINSTANCE hPrevInstance, LPSTR lpCmdLine, int nCmdShow) {
  const wchar_t CLASS_NAME[] = L"PrismaWindowClass";

  WNDCLASSEXW wc = {0};
  wc.cbSize = sizeof(WNDCLASSEXW);
  wc.lpfnWndProc = WindowProc;
  wc.hInstance = hInstance;
  wc.lpszClassName = CLASS_NAME;

  RegisterClassExW(&wc);

  HWND hwnd = CreateWindowExW(0, CLASS_NAME, L"Prisma Window", WS_OVERLAPPEDWINDOW, CW_USEDEFAULT,
                              CW_USEDEFAULT, 800, 600, NULL, NULL, hInstance, NULL);

  if (hwnd == NULL) {
    return 0;
  }

  ShowWindow(hwnd, nCmdShow);
  UpdateWindow(hwnd);

  MSG msg;
  while (GetMessageW(&msg, NULL, 0, 0)) {
    TranslateMessage(&msg);
    DispatchMessageW(&msg);
  }

  return 0;
}
