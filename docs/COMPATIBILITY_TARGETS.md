# Prisma — Windows application compatibility targets

Última actualización: 2026-08-07 America/Mexico_City.

Este documento convierte los objetivos de aplicaciones Windows en gates
reproducibles. No sustituye al [BACKLOG.md](BACKLOG.md): define qué significa
que cada demostración está realmente terminada.

## Niveles de evidencia

- **E0 — simulación de UI:** sirve para diseño, pero no cuenta como
  compatibilidad ni ejecución.
- **E1 — translate-only:** un host x86-64 decodifica el guest y produce ARM64,
  pero no ejecuta el resultado. Es un gate de compilación, no una demo real.
- **E2A — Linux ARM64 bajo QEMU:** un worker AArch64 ejecuta el código ARM64
  emitido por Prisma bajo emulación por software y entrega la evidencia a la
  app Android. Cuenta como evidencia funcional real del DBT, pero no valida
  todavía la integración JNI ni las APIs específicas de Android.
- **E2B — Android ARM64:** el APK `arm64-v8a` ejecuta el código ARM64 emitido
  por Prisma dentro de Android ARM64, emulado o físico. En un host Windows
  x86-64, el emulador oficial moderno rechaza los AVD ARM64; este nivel requiere
  otro hypervisor/emulador o un dispositivo ARM64.
- **E3 — dispositivo Android ARM64 físico:** gate posterior para rendimiento,
  temperatura, energía y compatibilidad dependiente de GPU/NPU.

Mientras no haya hardware E3 disponible, el core se desarrolla y valida en E2A.
El AVD x86-64 existente presenta la UI y consume la evidencia del worker local;
no se etiqueta como E2B ni se usa para publicar métricas de rendimiento.

### Checkpoint E2A — 2026-08-07

El probe mínimo PE32+ x86-64 se carga y traduce con el runtime de Prisma. Los
60 bytes ARM64 generados se ejecutan en AArch64 bajo QEMU y terminan con el
código esperado 42. La app Android muestra el registro `REAL` producido por el
worker, incluyendo arquitectura, entry point, tamaños y SHA-256. Este hito
valida el camino de ejecución del DBT; no implica todavía compatibilidad con
imports Win32, ventanas ni aplicaciones Windows de terceros.

Los artefactos oficiales ya adquiridos se registran en
[`tools/windows-apps/targets.lock.json`](../tools/windows-apps/targets.lock.json).
Los binarios permanecen fuera de Git y cada entrada fija URL, versión, tamaño,
SHA-256 y evidencia de firma o digest publicado.

## Target 1 — Oh My Posh x86-64 (CLI)

Oh My Posh es un motor de prompt ejecutable, no un emulador de terminal. Es el
primer objetivo porque ejercita una superficie Windows útil antes de introducir
WinUI, WebView o renderizado gráfico.

### Gate 1A — proceso no interactivo

- Usar una versión x86-64 Windows fijada por versión y SHA-256, con atribución
  de licencia. Los tests no descargan una versión flotante `latest`.
- Ejecutar `oh-my-posh.exe version` mediante el runtime real de Prisma en un
  dispositivo Android ARM64.
- Capturar `stdout`, `stderr` y el código de salida. Éxito: exit code 0, salida
  no vacía y ningún crash del host o guest.

### Gate 1B — render determinista

- Incluir una configuración local mínima `sample.omp.json`, sin red, Git ni
  herramientas externas.
- Ejecutar:

  ```text
  oh-my-posh.exe print primary --config sample.omp.json --shell uni
  ```

- Comparar el resultado normalizado con un golden UTF-8. Verificar por separado
  las secuencias ANSI y el fallback cuando no exista una Nerd Font.

### Gate 1C — sesión interactiva Android

- La vista Terminal de Compose presenta salida incremental y acepta teclado.
- El bridge soporta stdin/stdout/stderr, working directory, environment,
  resize, cancelación y exit status.
- Stop, Back, cierre del contenedor y reinicio ejecutan shutdown explícito:
  flush, cierre de pipes/handles/archivos, reap del proceso y unmap de memoria
  guest/JIT. Un segundo lanzamiento debe empezar aislado del anterior.

## Target 2 — aplicación GUI Windows pequeña

Notepad XP sigue siendo el gate gráfico de Fase 3. Debe abrir, recibir teclado,
redimensionarse, guardar/abrir un archivo dentro del overlay y cerrar sin dejar
procesos o recursos del prefix vivos.

## Target 3 — ChatGPT para Windows (stretch)

ChatGPT es un objetivo de compatibilidad avanzada y no bloquea los dos targets
anteriores. Antes de programar soporte específico se producirá una matriz
versionada de su paquete actual y de sus dependencias observadas: arquitectura,
MSIX/AppX, WinUI, WebView, red/TLS, autenticación, gráficos, audio y mecanismos
de actualización.

### Criterio de aceptación inicial

- El paquete lo obtiene legítimamente el usuario. Prisma no lo incluye ni lo
  redistribuye.
- La aplicación llega a su primera ventana utilizable en Android ARM64 y puede
  cerrarse limpiamente.
- La autenticación queda bajo control del usuario; los tests no almacenan ni
  automatizan credenciales.
- Un reporte de compatibilidad registra versión probada, dependencias, logs,
  fallos conocidos y recursos liberados durante teardown.

El paquete se obtiene únicamente mediante el enlace de descarga publicado por
OpenAI. Prisma registra la URL de origen, versión y SHA-256 local, pero no
redistribuye el instalador ni lo incorpora al repositorio.

## Target 4 — LibreOffice Desktop para Windows

El gate inicial es LibreOffice Writer x86-64 obtenido de libreoffice.org. Debe
abrir una ventana real, crear y guardar un `.odt` dentro del overlay, cerrarse,
volver a iniciar y abrir el mismo documento. Después se habilitan Calc e
Impress con fixtures pequeños y deterministas.

Además de los criterios GUI generales, el reporte debe cubrir fuentes,
impresión deshabilitada o aislada, portapapeles, diálogos de archivo, locale y
renderizado Skia/VCL. La suite no se considera compatible si únicamente aparece
el splash screen.

## Targets 5–10 — escalera adicional de escritorio

Cada artefacto se descarga desde el sitio oficial, se fija por versión y
SHA-256, y se conserva fuera de Git cuando su licencia no permita
redistribución:

1. **7-Zip x64:** ejecutar la CLI, abrir 7-Zip File Manager, crear y extraer un
   archivo de prueba.
2. **Notepad++ x64 portable:** editar, guardar y volver a abrir un archivo
   UTF-8; verificar teclado, menús y clipboard.
3. **SumatraPDF x64 portable:** abrir un PDF local, cambiar de página, hacer
   zoom y cerrar limpiamente.
4. **VLC x64:** abrir un clip local corto, reproducir audio/video y detenerlo;
   red y aceleración por hardware quedan fuera del primer gate.
5. **GIMP x64:** abrir una imagen, aplicar una operación básica y exportar PNG.
6. **KeePass 2 portable:** abrir una base de prueba sin secretos reales, crear
   una entrada, guardar y reiniciar. Este target añade el gate de runtime .NET.

Para todos: mostrar una ventana no basta. El test debe completar una acción
útil, verificar el artefacto resultante, registrar stdout/stderr/logs y repetir
el lanzamiento sin procesos, handles, mappings ni archivos temporales filtrados.

## Orden técnico

1. Loader PE, imports/relocations y bridge Wine.
2. Proceso de consola con pipes y lifecycle RAII.
3. Oh My Posh no interactivo y render determinista.
4. Terminal Compose interactiva y prueba stop/restart.
5. Notepad XP mediante el display server.
6. 7-Zip, Notepad++ y SumatraPDF como gates GUI incrementales.
7. LibreOffice Writer, VLC, GIMP y KeePass según sus dependencias.
8. Investigación del paquete ChatGPT y cierre de sus gaps de compatibilidad.

Este orden mantiene la causa de cada fallo observable: si Oh My Posh no arranca,
el problema está en el runtime Windows/console; si Notepad no aparece, está en
la ruta gráfica; si ChatGPT falla después, la matriz identifica la dependencia
moderna concreta que falta.
