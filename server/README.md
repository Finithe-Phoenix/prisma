# server — Prisma Cloud Services

**Lenguaje:** Rust.
**Framework:** Axum.
**Storage:** PostgreSQL + Cloudflare R2.
**P2P:** libp2p.

## Responsabilidad

Backend de **Pilar 4 (Translation cache distribuida)** + telemetría opt-in + Premium billing.

Endpoints principales:
- `GET /cache/{binary_hash}/{cpu_fingerprint}` — lookup de cache pre-computado.
- `POST /cache` — upload opt-in de cache generado por usuarios (verificado).
- `GET /compatibility/{game_id}` — compatibility database curada.
- `POST /telemetry/crash` — crash reports.
- `/p2p/tracker/*` — coordinación de nodos P2P.
- `/billing/*` — Stripe webhooks + Paddle.

## Zero-trust design

- Todos los caches se firman con Ed25519 antes de subirse.
- Los clientes verifican firma antes de mmap + exec.
- Telemetría estrictamente opt-in + anonimizada.
- P2P: IPs nunca se loguean, DHT no lista peers por user ID.

## Costos objetivo

- Fase 5 (beta 500 users): $50/mes.
- Fase 6 v1.0 (10k users): $200/mes.
- Año 1 post-v1.0 (50k users): $1000/mes.

Cloudflare R2 + Workers son la clave del bajo costo — egress gratis.

## No existe aún

Fase 2.5 (semanas 73-80) arranca con el cache service. Fase 5 añade el resto.
