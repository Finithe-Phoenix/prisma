# android — Prisma App

**Lenguaje:** Kotlin 2.0.
**UI:** Jetpack Compose.
**Min SDK:** 29 (Android 10 — por W^X awareness).
**Target SDK:** último al momento de release (dejando room para side-loading, no Play Store).

## Responsabilidad

La app Android como shell de usuario:

- Container manager (crear, listar, launch, eliminar).
- Shortcuts de juegos importados.
- Input mapper (gamepad, touch overlay).
- Integración con Steam/GOG/Epic (importación one-click).
- Configuración por-juego.
- Updates (vía GitHub Releases + Samsung Galaxy Store + Epic Games Store Android).
- Diagnóstico + crash reporting.

## APIs Android críticas

- **Performance Hint API** — para subir frequencies durante gameplay.
- **Game Mode API** — hint al sistema de que esto es un juego.
- **Storage Access Framework (SAF)** — importación de .exe sin pedir permisos peligrosos.
- **ANativeWindow + SurfaceView** — renderizado Vulkan del servidor X11 embebido.
- **AAudio** — bridge a WASAPI emulado.
- **Process VM Exec flags** — W^X configuration para permitir JIT.

## Por qué Compose (y no Flutter/React Native)

Acceso directo y sin wrappers a las APIs Android específicas que mencioné arriba. Flutter y RN las exponen tarde, mal o no las exponen. Para un emulador esto no es negociable.

## Preview local en Windows

La UI Compose ya se puede probar en el AVD nativo `Prisma_Device`. Desde la
raíz del repositorio:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File "C:\Users\daedg\OneDrive\Documentos\GitHub\prisma\scripts\run-prisma-android.ps1"
```

El lanzador:

- arranca o reutiliza `Prisma_Device`;
- usa renderizado software y desactiva Vulkan para evitar fallos del host;
- corrige la posición negativa de la ventana Qt en Windows 11;
- espera `sys.boot_completed=1`;
- instala `android/app/build/outputs/apk/debug/app-debug.apk`;
- abre `dev.prismaemu.app/.MainActivity`.

El build debug incluye `x86_64` solo para probar la UI en el emulador. El
objetivo de ejecución real de Prisma sigue siendo `arm64-v8a`. Los controles
que requieren JNI muestran una limitación explícita en la preview x86_64 hasta
que la biblioteca Rust/Android se empaquete en el APK.

El APK requiere JDK 17 para compilar. En esta máquina se validó con Gradle 8.9
y API 35 dentro de la imagen Docker local `android-emulator`.
