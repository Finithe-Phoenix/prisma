# Hoja de ruta activa de Prisma

La próxima entrega es una llamada futex del invitado capaz de sincronizar dos hilos mediante la tabla de espera existente. El corte de integración corresponde a `main` en `f0089a3`, revisado el 27 de noviembre de 2026. La épica de seguimiento es [#326](https://github.com/Finithe-Phoenix/prisma/issues/326). El [inventario canónico](BACKLOG.md#inventory-review-at-f0089a3) asigna las actividades a entregas de ingeniería o decisiones del titular.

## Base integrada

| Issue o cambio | Commit de integración | Resultado disponible |
|---|---|---|
| #315 · [PR #312](https://github.com/Finithe-Phoenix/prisma/pull/312) | `aa52753` | Instrucciones W1/W2 y RFLAGS persistente |
| #316 · [PR #327](https://github.com/Finithe-Phoenix/prisma/pull/327) | `b9653d5` | Estado de las llamadas de arranque por hilo |
| [PR #333](https://github.com/Finithe-Phoenix/prisma/pull/333) | `2396063` | La traducción de bloques fusionados respeta los bytes disponibles del invitado |
| #317 · [PR #328](https://github.com/Finithe-Phoenix/prisma/pull/328) | `f0089a3` | Tabla de espera portable, relectura del valor, límite de despertares y cierre |

## Hitos del runtime

| Orden | Entrega | Criterio de aceptación |
|---|---|---|
| 1 · [#334](https://github.com/Finithe-Phoenix/prisma/issues/334) | Conectar la syscall 202 a la tabla compartida mediante un contexto por hilo y una función explícita de lectura de memoria guest | Probar WAIT/WAKE y sus variantes privadas con dos hilos host, límite de despertares, discrepancia de valor, dirección inválida y cierre. Comandos, banderas y tiempos de espera sin soporte devuelven los errores especificados. |
| 2 · [#318](https://github.com/Finithe-Phoenix/prisma/issues/318) | Implementar clone y la gestión de hilos en Session según el [RFC 0022](rfc/0022-guest-threading-model.md) | Un productor y un consumidor guest se sincronizan mediante clone/futex en ARM64 real. Verificar registros, TLS y pila por hilo, arena y caché compartidas, liberación de recursos y TSan. |
| 3 · [#319](https://github.com/Finithe-Phoenix/prisma/issues/319) | Sustituir la imagen activa con execve y reingresar al traductor | Un segundo ejecutable arranca con su propio estado inicial. Una carga fallida conserva la primera imagen. Repetir la operación libera mappings, hilos, pilas y waiters. |
| 4 · Compatibilidad Linux | Ejecutar el corpus coreutils x86-64 en ARM64, clasificar los fallos por subsistema y corregir las regresiones reproducibles | Registrar versión del corpus, pruebas aprobadas y total. La meta es alcanzar al menos el 85 %. |

## Actividades paralelas

- **Semántica y paridad.** [#320](https://github.com/Finithe-Phoenix/prisma/issues/320) cubre RCL/RCR con contador CL. Ambos decodificadores ya procesan formas CL. Comparar sus resultados y flags con la referencia en los límites del contador y corregir discrepancias observadas. [#321](https://github.com/Finithe-Phoenix/prisma/issues/321) cubre los casos diferenciales C++/Rust, los serializadores y las definiciones Lean. Mantener en cero el presupuesto de `sorry`.
- **Rendimiento y CI.** [#322](https://github.com/Finithe-Phoenix/prisma/issues/322) cubre corpora reproducibles, el comando de reportes y resultados por motor sobre hardware comparable. [#323](https://github.com/Finithe-Phoenix/prisma/issues/323) cubre diagnósticos, sanitizers y fiabilidad en ARM64. Clasificar los fallos con el [manual de CI](CI_RUNBOOK.md).
- **Producto e investigación.** [#324](https://github.com/Finithe-Phoenix/prisma/issues/324) agrupa PE/Wine, Android, gráficos, AVF, NPU y caché distribuida firmada. Reutilizar el mapeo de PE, las reubicaciones y el enlace de importaciones. Completar Wine, arranque y parada de contenedores, overlay, JNI, pantalla y entrada hasta ejecutar Notepad en Android real. Evaluar gráficos y prototipos frente a referencias documentadas antes de beta y lanzamiento.
- **Decisiones del titular.** [#325](https://github.com/Finithe-Phoenix/prisma/issues/325) agrupa propiedad intelectual, licencias, organización y visibilidad del repositorio, dominio, dispositivos, privacidad y comunicación externa. Registrar decisión, fecha y dependencias. La [hoja plurianual](ROADMAP.md) contiene los hitos de beta, publicación y distribución.

## Integración y mantenimiento

Revisar el [PR #335](https://github.com/Finithe-Phoenix/prisma/pull/335) por subsistema. Frente a esta revisión, el cambio afecta a la cabecera de la API C, las cabeceras e implementaciones del dispatcher y del manejador de syscalls, y el backlog. Contrastar las afirmaciones sobre MMX, Lean, terminal y benchmarks con el árbol integrado. Vincular cada cambio aprovechable a su issue con una prueba específica y registrar el SHA al integrarlo.

Revisar cada actualización de dependencias con CI. [#311](https://github.com/Finithe-Phoenix/prisma/pull/311) actualiza la caché, [#314](https://github.com/Finithe-Phoenix/prisma/pull/314) prepara Python, [#336](https://github.com/Finithe-Phoenix/prisma/pull/336) sube artefactos, [#337](https://github.com/Finithe-Phoenix/prisma/pull/337) valida Markdown y [#338](https://github.com/Finithe-Phoenix/prisma/pull/338) ejecuta CodeQL. Abrir un PR por cambio revisable, adjuntar los resultados de CI y arquitectura pertinentes y registrar en el inventario su issue y SHA de integración.
