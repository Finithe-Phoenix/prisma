# third_party — Referencias de solo-lectura

Este directorio contiene **submódulos git** (cuando se añadan en Fase 1) de proyectos que leemos para referencia, NO para forkear.

## Política

- **NUNCA** editar código de `third_party/*`. Si se necesita un parche, documentarlo como patch file separado.
- Submódulos fijados a commit específico, no a branch. Actualización requiere PR + mención en `docs/research_notes.md`.
- Estos proyectos son **inspiración y comparación**, no base de código. El core de Prisma se escribe desde cero.

## Proyectos listados (futuro)

- `FEX/` — https://github.com/FEX-Emu/FEX (MIT) — referencia principal de DBT x86_64.
- `box64/` — https://github.com/ptitSeb/box64 (MIT) — referencia de dynarec ARM64.
- `wine/` — https://gitlab.winehq.org/wine/wine (LGPL) — usado como dependencia en Fase 3.
- `dxvk/` — https://github.com/doitsujin/dxvk (zlib) — usado como dependencia en Fase 4.
- `vkd3d-proton/` — https://github.com/HansKristian-Work/vkd3d-proton (LGPL) — dependencia en Fase 4.
- `mesa/` — https://gitlab.freedesktop.org/mesa/mesa (MIT) — para Turnip, dependencia en Fase 4.
- `vixl/` — https://github.com/Linaro/vixl (BSD-3-Clause) — ARM64 emitter library, dependencia en Fase 1.

## Nota de licencias

Wine, VKD3D-Proton son **LGPL**: linking dinámico OK, no estático. Ver `docs/legal/licensing.md` (futuro).
QEMU es **GPLv2 viral** — evitar en el core. Aislar como proceso separado si es necesario.
