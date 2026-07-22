# Hoja de ruta activa de Prisma

> Guía de ejecución para el programa de ingeniería actual. El inventario canónico de largo plazo permanece en [`BACKLOG.md`](BACKLOG.md); este documento es la vista priorizada y ordenada por dependencias.

## Centro de control

- Épica principal: [#326](https://github.com/Finithe-Phoenix/prisma/issues/326)
- Lote de integración actual: [PR #312](https://github.com/Finithe-Phoenix/prisma/pull/312)
- Arquitectura de hilos: [RFC 0022](rfc/0022-guest-threading-model.md)
- Cola detallada de trabajo: [`WORK_QUEUE.md`](WORK_QUEUE.md)

## Modelo de prioridad

| Prioridad | Significado | Política de integración |
|---|---|---|
| P0 | Bloquea el siguiente hito ejecutable | Debe atenderse antes de ampliar funciones no relacionadas |
| P1 | Seguimiento de corrección o arquitectura | Iniciar después de que aterrice el P0 que toque los mismos archivos |
| P2 | Calidad, rendimiento, infraestructura o investigación | Puede avanzar en paralelo cuando no exista conflicto de propiedad |
| Gobierno | Prerrequisito externo controlado por el propietario | Debe registrarse explícitamente; ingeniería no puede cerrarlo por sí sola |

## Fase A — estabilizar la línea base

### [#315 — Estabilizar e integrar el lote W1/W2](https://github.com/Finithe-Phoenix/prisma/issues/315)

**Resultado:** el PR #312 se integra con benchmarks, C++20, sanitizers, FFI, CodeQL, Rust, Lean, formato y documentación en verde.

Este paso es requisito para cualquier trabajo que dependa de RFLAGS persistente, CAS atómico real, WideDiv, PCMPxSTRx, PCLMULQDQ, BMI1 o F16C.

## Fase B — habilitar pthreads y glibc

Cadena de dependencias:

```text
#315 ──┬──> #316 syscalls de arranque ────┐
       └──> #317 futex WAIT/WAKE ─────────┼──> #318 Session multihilo + clone ──> #319 execve
                                        ┘
```

### [#316 — Syscalls de arranque de hilos para glibc](https://github.com/Finithe-Phoenix/prisma/issues/316)

Implementar `gettid`, `set_tid_address`, `set_robust_list` y un stub explícito de `rseq` sin modificar el comportamiento de ejecución monohilo.

### [#317 — Tabla de espera FUTEX_WAIT/FUTEX_WAKE](https://github.com/Finithe-Phoenix/prisma/issues/317)

Agregar la base portable requerida por mutexes y variables de condición de pthread, con revalidación del valor, límites de wakeup, limpieza y cobertura TSan.

### [#318 — Session multihilo y clone](https://github.com/Finithe-Phoenix/prisma/issues/318)

Ejecutar un hilo host por hilo guest sobre arena y caché compartidas, con un `CpuStateFrame` por hilo guest y una prueba productor/consumidor real en ARM64.

### [#319 — Sustitución de imagen mediante execve](https://github.com/Finithe-Phoenix/prisma/issues/319)

Reemplazar de forma segura el proceso guest actual, reconstruir el estado inicial y reingresar al traductor sin fugas de mappings, hilos, stacks o waiters.

## Fase C — cerrar brechas de corrección

### [#320 — RCL/RCR con conteo variable](https://github.com/Finithe-Phoenix/prisma/issues/320)

Completar las variantes controladas por `CL` después del PR #312 para minimizar conflictos en decoder y backend.

### [#321 — Paridad Rust/C++/Lean y corpus diferencial](https://github.com/Finithe-Phoenix/prisma/issues/321)

Automatizar la paridad del IR y serializadores, ampliar fixtures entre lenguajes y hacer que cualquier visitor o espejo semántico faltante falle claramente en CI.

## Fase D — confianza en rendimiento y entrega

### [#322 — Harness de benchmarks y líneas base](https://github.com/Finithe-Phoenix/prisma/issues/322)

Fortalecer pytest/Ruff/mypy, probar reportes, volver reproducibles los corpora, publicar artefactos y definir una política de regresiones que no sea inestable.

### [#323 — CI ARM64 e infraestructura de sanitizers](https://github.com/Finithe-Phoenix/prisma/issues/323)

Separar fallos de código y fallos de runner, conservar artefactos útiles, documentar recuperación y evaluar un runner ARM64 propio cuando exista justificación.

## Fase E — habilitadores de producto y programa

### [#324 — Investigación de Wine, gráficos, Android y NPU](https://github.com/Finithe-Phoenix/prisma/issues/324)

Convertir la investigación en notas de arquitectura, registros de decisión y criterios explícitos de avance sin desestabilizar el runtime principal.

### [#325 — Prerrequisitos legales, organización, dominio y comunidad](https://github.com/Finithe-Phoenix/prisma/issues/325)

Gestionar por separado decisiones de propiedad intelectual, visibilidad, organización, dominio, telemetría y colaboración externa.

## Acuerdo de trabajo

1. Usar una rama y un PR acotados por cada entrega revisable de forma independiente.
2. Definir criterios de aceptación y pruebas antes de implementar.
3. No integrar CI rojo sin una explicación formal.
4. Registrar el SHA de aterrizaje en el issue correspondiente.
5. Mantener `BACKLOG.md` completo y este documento enfocado en la ejecución activa.
6. Preferir cortes verticales pequeños que terminen en un resultado ejecutable u observable.
7. Evitar ediciones paralelas en los mismos archivos de decoder/backend salvo que exista propiedad explícita.

## Orden de ataque inmediato

1. Finalizar #315 e integrar el PR #312.
2. Implementar #316 como prerrequisito multihilo de menor impacto.
3. Construir #317 en paralelo únicamente donde no se crucen archivos.
4. Integrar #318 después de que ambas bases estén verdes bajo TSan.
5. Atender #320 después del PR #312 para evitar conflicto con el lote de flags persistentes.
6. Mejorar continuamente #321–#323 como líneas de calidad de apoyo.
