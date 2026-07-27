#include <windows.h>
#include <d3d9.h>
#include <math.h>

LPDIRECT3D9 d3d = NULL;
LPDIRECT3DDEVICE9 d3ddev = NULL;
LPDIRECT3DVERTEXBUFFER9 v_buffer = NULL;

struct CUSTOMVERTEX {
    FLOAT x, y, z;
    DWORD color;
};
#define CUSTOMFVF (D3DFVF_XYZ | D3DFVF_DIFFUSE)

void initD3D(HWND hWnd) {
    d3d = Direct3DCreate9(D3D_SDK_VERSION);
    D3DPRESENT_PARAMETERS d3dpp;
    ZeroMemory(&d3dpp, sizeof(d3dpp));
    d3dpp.Windowed = TRUE;
    d3dpp.SwapEffect = D3DSWAPEFFECT_DISCARD;
    d3dpp.hDeviceWindow = hWnd;
    d3dpp.BackBufferFormat = D3DFMT_UNKNOWN;
    d3dpp.EnableAutoDepthStencil = TRUE;
    d3dpp.AutoDepthStencilFormat = D3DFMT_D16;

    d3d->CreateDevice(D3DADAPTER_DEFAULT,
                      D3DDEVTYPE_HAL,
                      hWnd,
                      D3DCREATE_SOFTWARE_VERTEXPROCESSING,
                      &d3dpp,
                      &d3ddev);

    d3ddev->SetRenderState(D3DRS_LIGHTING, FALSE);
    d3ddev->SetRenderState(D3DRS_ZENABLE, TRUE);
}

void initGraphics() {
    CUSTOMVERTEX vertices[] = {
        // Front face
        { -1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(255, 0, 0) },
        {  1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(0, 255, 0) },
        { -1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(0, 0, 255) },
        { -1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(0, 0, 255) },
        {  1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(0, 255, 0) },
        {  1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(255, 255, 0) },
        
        // Back face
        { -1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(255, 0, 255) },
        { -1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(0, 255, 255) },
        {  1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(255, 255, 255) },
        {  1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(255, 255, 255) },
        { -1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(0, 255, 255) },
        {  1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(0, 0, 0) },
        
        // Top face
        { -1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(255, 128, 0) },
        {  1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(0, 255, 128) },
        { -1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(128, 0, 255) },
        { -1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(128, 0, 255) },
        {  1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(0, 255, 128) },
        {  1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(128, 255, 0) },
        
        // Bottom face
        { -1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(255, 0, 128) },
        {  1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(0, 128, 255) },
        { -1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(128, 255, 255) },
        { -1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(128, 255, 255) },
        {  1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(0, 128, 255) },
        {  1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(255, 255, 128) },
        
        // Left face
        { -1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(128, 128, 128) },
        { -1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(64, 64, 64) },
        { -1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(192, 192, 192) },
        { -1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(192, 192, 192) },
        { -1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(64, 64, 64) },
        { -1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(255, 255, 255) },
        
        // Right face
        {  1.0f,  1.0f, -1.0f, D3DCOLOR_XRGB(200, 100, 100) },
        {  1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(100, 200, 100) },
        {  1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(100, 100, 200) },
        {  1.0f, -1.0f, -1.0f, D3DCOLOR_XRGB(100, 100, 200) },
        {  1.0f,  1.0f,  1.0f, D3DCOLOR_XRGB(100, 200, 100) },
        {  1.0f, -1.0f,  1.0f, D3DCOLOR_XRGB(50, 50, 50) },
    };

    d3ddev->CreateVertexBuffer(36 * sizeof(CUSTOMVERTEX),
                               0,
                               CUSTOMFVF,
                               D3DPOOL_MANAGED,
                               &v_buffer,
                               NULL);

    VOID* pVoid;
    v_buffer->Lock(0, 0, (void**)&pVoid, 0);
    memcpy(pVoid, vertices, sizeof(vertices));
    v_buffer->Unlock();
}

void render_frame() {
    d3ddev->Clear(0, NULL, D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER, D3DCOLOR_XRGB(0, 40, 100), 1.0f, 0);

    d3ddev->BeginScene();
    d3ddev->SetFVF(CUSTOMFVF);

    static float index = 0.0f; index+=0.05f;

    D3DMATRIX matRotateY;
    ZeroMemory(&matRotateY, sizeof(matRotateY));
    matRotateY.m[0][0] = cosf(index); matRotateY.m[0][2] = -sinf(index);
    matRotateY.m[1][1] = 1.0f;
    matRotateY.m[2][0] = sinf(index); matRotateY.m[2][2] = cosf(index);
    matRotateY.m[3][3] = 1.0f;

    D3DMATRIX matRotateX;
    ZeroMemory(&matRotateX, sizeof(matRotateX));
    matRotateX.m[0][0] = 1.0f;
    matRotateX.m[1][1] = cosf(index); matRotateX.m[1][2] = sinf(index);
    matRotateX.m[2][1] = -sinf(index); matRotateX.m[2][2] = cosf(index);
    matRotateX.m[3][3] = 1.0f;
    
    D3DMATRIX matWorld;
    for(int i=0; i<4; i++) {
        for(int j=0; j<4; j++) {
            matWorld.m[i][j] = matRotateX.m[i][0]*matRotateY.m[0][j] +
                               matRotateX.m[i][1]*matRotateY.m[1][j] +
                               matRotateX.m[i][2]*matRotateY.m[2][j] +
                               matRotateX.m[i][3]*matRotateY.m[3][j];
        }
    }
    
    d3ddev->SetTransform(D3DTS_WORLD, &matWorld);

    D3DMATRIX matView;
    ZeroMemory(&matView, sizeof(matView));
    matView.m[0][0] = 1.0f;
    matView.m[1][1] = 1.0f;
    matView.m[2][2] = 1.0f;
    matView.m[3][2] = 5.0f;
    matView.m[3][3] = 1.0f;
    d3ddev->SetTransform(D3DTS_VIEW, &matView);

    D3DMATRIX matProj;
    ZeroMemory(&matProj, sizeof(matProj));
    matProj.m[0][0] = 1.81f;
    matProj.m[1][1] = 2.41f;
    matProj.m[2][2] = 1.01f; matProj.m[2][3] = 1.0f;
    matProj.m[3][2] = -1.01f;
    d3ddev->SetTransform(D3DTS_PROJECTION, &matProj);
    
    d3ddev->SetStreamSource(0, v_buffer, 0, sizeof(CUSTOMVERTEX));
    d3ddev->DrawPrimitive(D3DPT_TRIANGLELIST, 0, 12);

    d3ddev->EndScene();
    d3ddev->Present(NULL, NULL, NULL, NULL);
}

LRESULT CALLBACK WindowProc(HWND hWnd, UINT message, WPARAM wParam, LPARAM lParam) {
    switch(message) {
        case WM_DESTROY: {
            PostQuitMessage(0);
            return 0;
        } break;
    }
    return DefWindowProc(hWnd, message, wParam, lParam);
}

int WINAPI WinMain(HINSTANCE hInstance, HINSTANCE hPrevInstance, LPSTR lpCmdLine, int nCmdShow) {
    HWND hWnd;
    WNDCLASSEX wc;
    ZeroMemory(&wc, sizeof(WNDCLASSEX));
    wc.cbSize = sizeof(WNDCLASSEX);
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = WindowProc;
    wc.hInstance = hInstance;
    wc.hCursor = LoadCursor(NULL, IDC_ARROW);
    wc.hbrBackground = (HBRUSH)COLOR_WINDOW;
    wc.lpszClassName = "WindowClass";
    RegisterClassEx(&wc);

    hWnd = CreateWindowEx(0, "WindowClass", "Cube3D",
                          WS_OVERLAPPEDWINDOW, 300, 300, 800, 600,
                          NULL, NULL, hInstance, NULL);
    ShowWindow(hWnd, nCmdShow);
    initD3D(hWnd);
    initGraphics();

    MSG msg;
    while(TRUE) {
        while(PeekMessage(&msg, NULL, 0, 0, PM_REMOVE)) {
            TranslateMessage(&msg);
            DispatchMessage(&msg);
        }
        if(msg.message == WM_QUIT)
            break;
        render_frame();
    }
    return msg.wParam;
}
