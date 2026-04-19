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

## No existe aún

Fase 3 (semanas 105-112) arranca la GUI mínima. Fase 5 (semanas 141-152) es polish + UX completo.
