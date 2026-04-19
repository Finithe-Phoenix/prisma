# Prisma RFCs — proceso de decisiones arquitectónicas

## Qué es un RFC

Un **RFC (Request For Comments)** es un documento que propone y justifica una decisión arquitectónica con impacto cross-subproyecto o con coste alto de revertir.

En Prisma, RFCs son:
- **Contratos:** lo que acordamos hacer y por qué.
- **Historial:** cuando dentro de 2 años alguien pregunte "por qué X", el RFC responde.
- **Material para papers:** las decisiones bien documentadas se convierten en sections de papers.

## Cuándo escribir un RFC

Escribir RFC cuando la decisión:
- Afecta a múltiples subproyectos (ej: formato del translation cache on-disk).
- Es costosa de revertir (ej: elegir formato de IR).
- Tiene implicaciones de correctness (ej: política de memoria del DBT).
- Establece un invariante que otros componentes asumen (ej: layout del `CpuStateFrame`).

**NO** escribir RFC para:
- Implementación de un bugfix local.
- Refactor contenido en un subproyecto.
- Decisiones de estilo de código (ver `CLAUDE.md` y config files).

## Numeración

RFCs se numeran secuencialmente a 4 dígitos: `0001-short-slug.md`, `0002-...`. Cuando abres uno nuevo, usa el siguiente número disponible. No reutilizar números (los rechazados se mantienen con su número).

## Estados

En la frontmatter de cada RFC:

- **`draft`** — en escritura, no se ha tomado decisión aún.
- **`proposed`** — listo para revisión. Se espera feedback por al menos 48 horas antes de pasar a `accepted`.
- **`accepted`** — decisión tomada, se implementa conforme al documento.
- **`superseded`** — reemplazado por RFC posterior. La frontmatter linkea al nuevo.
- **`rejected`** — evaluado y rechazado. Se conserva porque el razonamiento es valioso.

## Plantilla

```markdown
---
id: NNNN
title: <Título corto imperativo>
status: draft | proposed | accepted | superseded | rejected
authors: [Danny]
created: YYYY-MM-DD
updated: YYYY-MM-DD
supersedes: []      # lista de IDs (ej: [0003])
superseded_by: null # ID del RFC que lo reemplaza, si aplica
---

# RFC NNNN: <Título>

## Summary

Un párrafo. Qué y por qué.

## Motivation

Por qué esta decisión importa. Qué problema resuelve. Qué pasa si no la tomamos.

## Context

Contexto necesario para entender las opciones. Incluir:
- Restricciones (performance, memoria, licencia, timeline).
- Trabajo previo (otros DBTs, papers, referencias en `docs/research_notes.md`).
- Invariantes del sistema que la decisión debe respetar.

## Considered alternatives

Listar las opciones reales evaluadas. Para cada una:
- Qué es.
- Ventajas.
- Desventajas.
- Por qué se descartó (si aplica).

Mínimo 2 alternativas. Si solo hay una, probablemente no necesitas RFC.

## Decision

Qué se decide. Debe ser accionable y verificable.

## Consequences

- **Beneficios:** qué ganamos.
- **Costes:** qué pagamos (complejidad, mantenance, dependencias).
- **Reversibilidad:** ¿cómo revertir? ¿qué código/datos se romperían?

## Implementation notes

(Opcional) notas sobre cómo implementar, orden sugerido, files afectados. No debe ser código — pertenece al PR.

## Open questions

Lo que el RFC no resuelve pero necesitamos responder eventualmente.

## References

- Links a papers, code, issues, discusiones.
```

## RFCs activos

Ver los archivos `0001-*.md` en adelante en este directorio.
