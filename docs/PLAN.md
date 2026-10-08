# SoftLadder — Plan de proyecto

> Documento de trabajo **en español** (es el plan que estamos construyendo juntos).
> El código, el `README`, los ADRs y los comentarios del repositorio van en **inglés**,
> que es lo habitual en un repositorio público.
>
> Estado: **borrador v0.1 — M0 (esqueleto) ejecutado**. Última revisión: ver `git log`.

---

## 1. Visión

**SoftLadder** es un clon de [ClassicLadder](https://github.com/MaVaTi56/classicladder) escrito en
Rust: editor de lógica de escalera (ladder) y de lenguaje secuencial (SFC/Grafcet), más un runtime
PLC determinista, con **mejor UI/UX**, **mejor ingeniería** (testeable, seguro, multiplataforma) y
**compatibilidad real** con los proyectos `.clprj` existentes.

Propuesta de valor en una frase:

> Todo lo que ClassicLadder hace, pero con un editor moderno, simulación/ensayo integrado,
> tests automatizados de la lógica y ejecución headless — sin perder tus proyectos actuales.

### Objetivos

1. **Paridad funcional** con ClassicLadder en el dominio ladder + SFC + variables + IO.
2. **Interoperabilidad**: importar y exportar proyectos ClassicLadder (ida y vuelta verificada).
3. **UX de primer nivel**: edición tipo IDE (undo/redo ilimitado, multi-selección, paleta de
   comandos, cross-reference, autocompletado de variables), no un formulario GTK.
4. **Verificabilidad**: simulador integrado, escenarios de test `.sltest` ejecutables en CI,
   grabación/replay de ciclos de scan para post-mortem.
5. **Multiplataforma**: Linux, macOS, Windows y (más adelante) WebAssembly en navegador.
6. **Seguridad y robustez**: parsers sin `unsafe`, evaluación de expresiones en sandbox, ningún
   `system()`/shell, fuzzing de todos los formatos de entrada.

### No-objetivos (por ahora)

- Certificación IEC 61131-3 completa ni PLCopen conformance oficial.
- Certificación de seguridad funcional (SIL/PL).
- Garantía de tiempo real duro: damos determinismo de ciclo y medimos jitter; el tiempo real
  estricto queda como modo opcional sobre Linux con `PREEMPT_RT`/`SCHED_FIFO`, no como promesa.
- Reemplazar a LinuxCNC: nos integramos con él.

---

## 2. Punto de partida: qué hay hoy en ClassicLadder

Inventario verificado sobre el clon en `../classicladder` (v0.9.113+, 138 archivos en `src/`, 39
proyectos de ejemplo en `projects_examples/`).

| Área | Implementación actual |
| --- | --- |
| Modelo | `classicladder.h`: `StrRung` con grilla fija `RUNG_WIDTH 12 × RUNG_HEIGHT 8`, `StrElement`, tamaños de arrays por proyecto (`plc_sizeinfo_s`) |
| Motor | `calc.c` (ladder), `calc_sequential.c` (SFC), `arithm_eval.c` (expresiones), `vars_access.c` (lectura/escritura por tipo de variable) |
| Elementos | Contactos NO/NC, flancos subida/bajada, bobinas OUT/SET/RESET, TIMER IEC (ON/OFF/PULSE), TIMER/MONOSTABLE legados, COUNTER, COMPARE, OPERATE (asignación), REGISTER (FIFO/LIFO), JUMP, CALL a subrutina |
| Variables | `%B` bits, `%W` words, `%I`/`%Q` físicas, `%IW`/`%QW` analógicas, `%TM`/`%C`/`%M`/`%X` (etapas)/`%S` (sistema), `%QLED`; tabla de símbolos de 10 caracteres |
| Secciones | Main ordenables + subrutinas numeradas (`StrSection`) |
| SFC | `sequential.h`: 5 páginas de 32×32, 128 etapas, 256 transiciones, AND/OR, divergencias |
| UI | GTK2/GTK3: toolbar de elementos, popup con click derecho, ventanas separadas de variables, símbolos, logs, config, monitor |
| IO | Puerto paralelo directo, Comedi, GPIO RPi (wiringPi), GPIO Atmel SAM, Modbus TCP/RTU **maestro**, Modbus TCP **esclavo** (FC 1,2,3,4,5,6,15,16) |
| Monitor | Protocolo propio por UDP/serial/módem: ver rungs, escribir variables, transferir proyecto, estado remoto, reset, hora |
| Proyecto | Contenedor de texto `_FILES_CLASSICLADDER` + `_FILE-<nombre>` …, `.clp`/`.clprj` planos y `.clprjz` con gzip; dentro: `general.txt`, `rungs.txt`, `sections.txt`, `symbols.txt`, `vars.txt`, `logs.txt`, `sequential_*.txt`, `alarms.txt` con encabezados `#VER=x.y` |
| Alarmas | 8 slots SMS/email vía módem GSM/comando externo `mailsend` |
| RT | Xenomai (usuario), RTLinux/RTAI (abandonado) |
| Idiomas | gettext (`po/`): en, fr, es, … |

### Dolores conocidos (del propio `docs/TODO.txt` y de la arquitectura)

- **Sin undo/redo** en el editor; copiar/pegar por partes es un flujo de dos pasos poco descubrible.
- **Sin refactor**: la tabla de símbolos es de 10 caracteres y renombrar una variable no propaga.
- **Sin tests**: no hay forma automática de validar la lógica de un programa.
- **Sin diff útil**: el `.clprj` es legible pero `general.txt`/`rungs.txt` no están pensados para git.
- Grilla rígida 12×8 por rung: rungs anchos requieren partir la lógica.
- Expresiones limitadas a 50 caracteres, sin punto flotante, sin funciones matemáticas.
- Errores de ejecución (p. ej. división por cero) no aíslan el rung de forma sistemática.
- Monitor sin autenticación ni cifrado; UI de variables en ventanas separadas, sin scope/tendencia.
- Modbus esclavo con mapa de registros **no parametrizable** (espeja `%B`/`%W`).
- Todo el estado del proyecto en variables globales (`InfosGene`), difícil de testear o instanciar dos veces.
- Configuración por compilación (`Makefile`) para features y targets embebidos.

---

## 3. Diferenciadores: ClassicLadder → SoftLadder

| # | ClassicLadder hoy | SoftLadder |
| --- | --- | --- |
| 1 | Sin undo/redo | **Undo/redo ilimitado** por comando, con historial visual por sección |
| 2 | Selección simple | **Multi-selección** (rubber band), mover/copiar/duplicar bloques, alineación y auto-ruteo de ramas paralelas |
| 3 | Editar rung por rung | **Edición directa sobre canvas** con zoom/pan infinito, snap a grilla, hover, drag&drop desde paleta |
| 4 | Sin simulación real | **Panel de simulación** integrado: interruptores, pulsadores, LEDs, sliders y gauges analógicos, guardable en el proyecto |
| 5 | Solo mirar valores | **Scope/osciloscopio** con tendencias (egui_plot), cursores y exportación CSV |
| 6 | Sin tests | **`.sltest`**: escenarios con línea de tiempo de entradas y aserciones de salida; `softladder test` en CI |
| 7 | Sin refactor | **Renombrar variable/símbolo en todo el proyecto**, cross-reference ("usado por", "escribe", "lee") |
| 8 | Símbolos de 10 chars | Símbolos largos, comentarios, tipos, unidades, valores por defecto, validación de nombres |
| 9 | Sin diff | Formato nativo **estable y ordenado** (JSON pretty determinista) pensado para `git diff` legible |
| 10 | Sin post-mortem | **Flight recorder**: buffer circular de los últimos N ciclos (entradas, estado, salidas) con dump y replay |
| 11 | Errores opacos | **Panel de problemas** con diagnóstico estructurado (código, severidad, rung, elemento, quick-fix) |
| 12 | Monitor sin auth | Protocolo **JSON/CBOR sobre TCP + WebSocket**, TLS opcional, roles read-only/read-write, token |
| 13 | HMI = GTK | **Dashboard web** servido por el runtime para monitorear/forzar desde el navegador o el móvil |
| 14 | Alarmas por SMS/módem | **Alarmas/eventos** con severidad, acuse, journal SQLite, webhooks/email; sin módems |
| 15 | Modbus esclavo fijo | **Mapa Modbus configurable** (offset, tipo, escala, byte order) + maestro con reintentos y supervisión |
| 16 | Solo entero | Palabra `i32`, `i64` y **real `f64`**, con promoción y errores de tipo detectados en edición |
| 17 | `arithm_eval` de 50 chars | **Parser tipado propio** (Pratt), sin `eval`, con funciones matemáticas, comparaciones encadenables y división segura |
| 18 | Grilla 12×8 | Rungs de **ancho variable** con auto-layout; la grilla sigue existiendo (semántica ladder) pero deja de ser una jaula |
| 19 | Config por #define | Features por **crate features** y configuración en runtime (TOML) |
| 20 | Estado global | Núcleo con `Project`/`Runtime` **instanciables** (varios PLCs en un proceso para tests) |
| 21 | GTK Linux | **egui/eframe**: Linux, macOS, Windows, WASM; tema claro/oscuro, HiDPI, i18n con Fluent |
| 22 | Exportar impresión por GNOME | Export **PNG/SVG/PDF** de rungs y del proyecto completo |

Extras que igualan el ecosistema actual: **ST/IL/FBD** y bloques **PID/PWM/escalado** (M9),
import/export **PLCopen XML** para hablar con CODESYS/OpenPLC.

---

## 4. Arquitectura

Workspace Cargo con crates de responsabilidad única; el núcleo no conoce ni IO ni UI.

```
softladder/
├── crates/
│   ├── softladder-core/      # dominio puro: modelo, scan engine, FBs, SFC, expresiones, diagnósticos
│   ├── softladder-project/   # formato nativo (.slprj) + import/export ClassicLadder + PLCopen XML
│   ├── softladder-runtime/   # scheduler de scan, máquina de estados, grabación/replay, alarmas
│   ├── softladder-edit/      # comandos de edición, undo/redo y banco de simulación (sin UI)
│   ├── softladder-io/        # traits de IO + drivers: sim, Modbus TCP/RTU, gpiod, LinuxCNC HAL
│   ├── softladder-monitor/   # protocolo online (JSON/CBOR + WS), servidor del dashboard web
│   ├── softladder-ui/        # editor egui/eframe (binario principal)
│   └── softladder-cli/       # binario headless: run, test, lint, migrate, import, export
├── docs/                     # PLAN, ARCHITECTURE, COMPAT, FORMAT, ADRs
├── examples/                 # proyectos .slprj y escenarios .sltest de ejemplo
├── testdata/                 # corpus dorado (proyectos ClassicLadder) + snapshots
├── fuzz/                     # cargo-fuzz para importadores y parser de expresiones
└── .github/workflows/ci.yml  # fmt, clippy, test, build multiplataforma, coverage
```

### Grafo de dependencias (regla dura)

```
core  ←  project  ←  runtime  ←  io
  ↑         ↑           ↑        ↑
  │         └── edit ───┘        │
  └──────── ui ──────────────────┘
             cli ────────────────┘
```

- `softladder-core` **no** depende de ningún otro crate del workspace ni de IO/UI/red.
- `softladder-edit` concentra *lo que el editor hace* (comandos, historial, ficheros, banco de
  simulación) para que todo eso se testee sin abrir una ventana; `softladder-ui` solo dibuja y
  traduce entrada en `Command`s.
- `#![forbid(unsafe_code)]` en `core`, `project`, `runtime`, `monitor`, `ui`, `cli`.
  `unsafe` solo en crates `-sys` de FFI (gpiod, HAL), aislado y documentado.
- El tiempo entra al núcleo como **parámetro** (`now: Instant`/tick), nunca llamando al reloj dentro:
  así la lógica es reproducible en tests.

Detalles en [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## 5. Modelo de dominio (núcleo)

- **Variables**: espacio de nombres IEC con alias de compatibilidad:
  `%M` (bits memoria), `%MW` (palabras), `%I`/`%Q` (digitales físicas), `%IW`/`%QW` (analógicas),
  `%TM` (temporizadores IEC), `%C` (contadores), `%R` (registros FIFO/LIFO), `%X` (etapas SFC),
  `%S` (sistema: reloj, estado, scan time…), `%QLED`.
  Alias heredados aceptados al importar: `%B`→`%M`, `%W`→`%MW`, `%TM`/`%C`/`%X` se mantienen.
- **Tipos**: `Bit`, `Word(i32)`, `DWord(i64)`, `Real(f64)`. Coerción explícita, diagnósticos de tipo.
- **Índices**: `VarRef { kind, index, index_expr?, bit? }` (indexado por variable, como `%MW[%MW0]`,
  y selección de bit `.n`, como `%MW20.3`) — dos ítems del TODO original que aquí son nativos.
  Los accesos derivados usan sufijo `.V` (`%TM0.V`, `%C0.V`).
- **Elementos**: contactos (NO/NC/flanco), bobinas (OUT/OUT-/SET/RESET/JUMP/CALL),
  FBs: `TON/TOF/TP`, `CTU/CTD/CTUD`, `R_TRIG/F_TRIG`, `RS/SR`, `COMPARE`, `OPERATE`,
  `REGISTER` (FIFO/LIFO), `SEL/MUX/LIMIT`, `SCALE/NORM`, `PID`, `PWM`, `RTC`, y contadores rápidos (stub).
- **Secciones**: `Main` (orden explícito) y `SubRoutine` (numerada), con `Language = Ladder | Sfc`.
- **SFC**: etapas (con flag inicial), transiciones con condición booleana/expresión, divergencias
  Y/O y convergencias, acciones con calificadores IEC (`N,S,R,L,D,P,SD,DS,SL`), temporizadores de etapa,
  páginas de tamaño libre.
- **Diagnósticos**: `Diagnostic { severity, code, section, rung, element, message, quick_fix }`.
  Un rung con error de ejecución se marca y **no** tumba el scan (equivalente al `RungInError` pero explícito).

## 6. Motor de scan

Ciclo determinista, sin asignaciones en el camino caliente:

1. `read_inputs()` → imagen de entradas (driver).
2. Resolver `Main` en orden, con pila de llamadas a subrutinas (límite de profundidad) y saltos validados.
3. Resolver SFC activo por página.
4. `write_outputs()` desde la imagen de salidas.
5. Actualizar estadísticas (tiempo, jitter, ciclos perdidos) y disparar alarmas/eventos.

Garantías: orden de ejecución estable y documentado; expresiones sin pánico (división por cero,
overflow con `checked_*`, índices fuera de rango → diagnóstico); `run / stop / single-cycle / freeze`;
hot-reload del proyecto en el borde de ciclo; grabación de entradas para **replay exacto**.

---

## 7. Formato de proyecto e interoperabilidad

### Nativo — `.slprj`

- JSON determinista (claves ordenadas, sin `HashMap` en la serialización) → diffs legibles en git.
- Variante comprimida `.slprjz` (zstd) para targets embebidos.
- `schema_version` + migraciones encadenadas y testeadas (`v1 → v2 → …`).
- Ejemplo: `examples/traffic_light.slprj`.

### Compatibilidad ClassicLadder — ver [`COMPAT.md`](COMPAT.md)

- **Import**: `.clp`, `.clprj` y `.clprjz` (contenedor gzip `_FILES_CLASSICLADDER`, partes
  `_FILE-<nombre>`…, archivos internos `general.txt`, `rungs.txt`, `sections.txt`, `symbols.txt`,
  `vars.txt`, `logs.txt`, `sequential_*.txt`, con cabeceras `#VER=x.y`).
- **Export**: emite el formato 3.0 (rungs) / 2.0 (sections) para que un ClassicLadder ya instalado
  pueda consumir el proyecto (drop-in).
- **Corpus dorado**: los 39 proyectos de `../classicladder/projects_examples/` son la suite de
  aceptación. Criterio: *parsear → normalizar → reexportar → reparar* debe ser estable, y el scan de
  SoftLadder debe reproducir el comportamiento del motor C en los casos cubiertos.
- Los elementos que no mapean se reportan como avisos, nunca se descartan en silencio.

---

## 8. IO y runtime

- Trait `IoDriver` (imagen de entradas/salidas, digital y analógica, con timestamp y calidad).
- Drivers: `sim` (panel + scripting), `modbus` (maestro TCP/RTU con reintentos y supervisión;
  esclavo TCP/RTU con **mapa configurable**), `gpio` (libgpiod v2 en Linux, `rppal` en RPi),
  `hal` (pines LinuxCNC vía `-sys` con feature `linuxcnc`), `mqtt`/`opcua` (posteriores).
- Supervisión: timeouts, reconexión, política de **estado seguro** de salidas ante fallo de driver.
- Tiempo real opcional: `SCHED_FIFO` + `mlockall` + CPU pinning (feature `rt`, Linux), medición de
  jitter p50/p99, histograma y contador de ciclos perdidos.
- **Flight recorder** y **replay**: reproducir un incidente con las entradas grabadas.

## 9. Monitor online y dashboard

- Protocolo nuevo: mensajes JSON (o CBOR) sobre TCP y WebSocket; TLS opcional; token con roles.
- Capacidades: estado, variables, forzar/liberar, tendencias, alarmas, subir/bajar proyecto, reset.
- Dashboard web estático servido por el runtime: monitoreo y forzado desde navegador/móvil.
- Compatibilidad: ADR pendiente sobre si implementar además el protocolo binario del monitor de
  ClassicLadder para interoperar con herramientas existentes.

## 10. UI/UX (egui/eframe)

- **Layout**: docking (`egui_dock`) — árbol de proyecto, lista de secciones, canvas central,
  y paneles: símbolos, variables/watch, scope, cross-reference, problemas, logs, simulación.
- **Canvas**: zoom/pan, grilla infinita, culling y cache de teselado (objetivo: 5.000 elementos a 60 fps),
  power-flow animado con el cable energizado, valores en vivo dentro del elemento, resaltado de fallos.
- **Edición**: undo/redo ilimitado, multi-selección, drag&drop desde paleta, duplicar ramas,
  atajos configurables, paleta de comandos (Cmd/Ctrl+Shift+P), edición inline de variables con
  autocompletado y creación al vuelo de símbolos.
- **Simulación**: panel tipo HMI (switches, pulsadores, LEDs, sliders, gauges) guardado en el proyecto,
  reproductor de escenarios con scrub en la línea de tiempo, comparación esperado vs real.
- **Calidad de vida**: autosave + recuperación ante crash, proyectos recientes, plantillas,
  snapshots versionados con visor de diff, i18n (Fluent: es/en/fr), tema claro/oscuro, HiDPI,
  export PNG/SVG/PDF y `Ctrl+P` para imprimir el programa.
- **Accesibilidad**: navegación completa por teclado, foco visible, escalado de fuente, contraste.

## 11. Testing, QA y CI

- `cargo nextest` + `insta` (snapshots) + `proptest` (parsers, round-trip) + `criterion` (benchmarks).
- `cargo-fuzz`: importador `.clprj`, parser de expresiones, lector de `.slprj`.
- **Corpus dorado** de los proyectos ClassicLadder y **tests diferenciales** contra el motor C
  (harness opcional que compila `calc.c` como oráculo) — marcado como *stretch*, en ADR.
- `.sltest`: escenarios de lógica ejecutables en CI (el equivalente a tests unitarios para el PLC).
- CI GitHub Actions: `fmt --check`, `clippy -D warnings`, `test`, build Linux/macOS/Windows,
  `cargo-deny`, cobertura con `llvm-cov`, y job WASM (a partir de M6).
- Convenciones: commits convencionales, ADRs para decisiones, `MSRV` fijada, semver por crate.

---

## 12. Roadmap por hitos

Cada hito termina con criterios de aceptación verificables. **M0 ya está ejecutado.**

| Hito | Contenido | Criterio de aceptación |
| --- | --- | --- |
| **M0** ✅ | Repositorio, workspace Cargo, CI, plan y ADRs | `cargo fmt/clippy/test` en verde y CI corriendo en GitHub |
| **M1** ✅ | Núcleo: variables v2 con accesores, power flow por columna, FBs completos (temporizadores, contadores, registros), saltos y subrutinas, `lint` estructural, tiempo simulado determinista, migración v1→v2 | 167 tests en verde; el ejemplo se ejecuta por CLI con salida idéntica entre ejecuciones; `lint` sin diagnósticos |
| **M2** ✅ | Editor egui: paleta de elementos, canvas con power flow en vivo, undo/redo por comandos, banco de simulación persistido, panel de problemas, `softladder-edit` sin UI | El test de aceptación `m2_acceptance.rs` abre el semáforo, lo simula (arranque, enclavamiento, stop, temporizador), lo edita, lo guarda y lo reabre idéntico; 330 tests en verde |
| **M3** ✅ | Import/export ClassicLadder + corpus dorado | Los 41 proyectos del corpus importan sin pánico (272 rungs, 9.273 elementos); `import → export → import` es punto fijo; la segunda exportación es byte a byte idéntica; las partes no modeladas sobreviven intactas; un test de comportamiento prueba que el temporizador importado recibe su enable con preset y base correctos |
| **M4** | SFC/Grafcet: modelo, motor y editor | Los ejemplos `example_sequential*.clprj` se ejecutan igual que en la referencia |
| **M5** | IO: sim scripting, Modbus TCP maestro/esclavo, luego RTU, con mapa configurable | Test de integración con servidor Modbus simulado; esclavo expone el mapa configurado |
| **M6** | Monitor online + dashboard web + alarmas/journal | Ver y forzar variables desde el navegador; journal consultable; forzado auditado |
| **M7** | IO físico (gpiod/rppal) + puente LinuxCNC HAL | Ejecución sobre GPIO real en RPi y lectura/escritura de pines HAL |
| **M8** | `.sltest`, flight recorder, replay, fuzzing, documentación de usuario | Escenario de test en CI; incidente reproducido desde una grabación |
| **M9** | Extras IEC: ST/IL/FBD, PID/PWM/escalado, PLCopen XML, WASM, empaquetado (deb/Homebrew/MSI/AppImage) | PID y un bloque ST funcionando; paquete instalable por plataforma |

Hitos de calidad transversales: cada hito añade sus tests, sus docs y su entrada de `CHANGELOG.md`.

## 13. Riesgos y mitigación

| Riesgo | Impacto | Mitigación |
| --- | --- | --- |
| Alcance gigantesco (ClassicLadder son ~20 años) | Alto | Hitos cerrados, lista de recorte explícita, "no-objetivos" escritos |
| Quirks no documentados del formato `.clprj` | Medio | Corpus dorado de 39 proyectos + fuzzing + tests diferenciales contra el C |
| Determinismo en kernels no-RT | Medio | Medir y publicar jitter; estados safe; no prometer RT duro |
| Rendimiento del canvas egui con programas grandes | Medio | Culling, virtualización, cache de teselado, benchmarks en CI |
| FFI de gpiod/LinuxCNC | Medio | Crates `-sys` aislados, features off por defecto, tests con simulador |
| Licencia y procedencia del código | Alto | ADR-0001: reimplementación limpia; no traducir código C; atribución a ClassicLadder |
| Falta de hardware para probar | Medio | Modbus en loopback + simulador + grabaciones reales como fixtures |

## 14. Licencia y procedencia

- Código propio: **MIT OR Apache-2.0** (pendiente de confirmar en ADR-0001).
- ClassicLadder es LGPL-2.1+/LGPL-3: se respeta su autoría y se le acredita en `NOTICE`.
- Política: se reimplementa a partir del **formato y del comportamiento observable**, no se traduce
  código C línea a línea. Cualquier fragmento derivado se marcaría explícitamente.

## 15. Convenciones de ingeniería

- `rustfmt` + `clippy -- -D warnings`, sin excepciones en CI.
- Errores con `thiserror` en librerías y `anyhow` en binarios; nunca `unwrap()` en camino de runtime.
- Logging estructurado con `tracing` (`--log-format json`).
- Nombres de dominio en inglés; identificadores IEC tal cual (`%M0`, `TON`, `CTUD`).
- Todo cambio de comportamiento va acompañado de su test; todo formato nuevo, de su fuzz target.
- `docs/adr/NNNN-titulo.md` para decisiones que no se deducen del código.

## 16. Estado actual del repositorio (M0)

- Workspace Cargo con los 7 crates, compilando y con tests mínimos.
- CI en GitHub Actions (fmt, clippy, test, build cruzado).
- Documentación: este plan, `SEMANTICS.md` (especificación normativa del motor), `ARCHITECTURE.md`,
  `COMPAT.md`, `FORMAT.md`, `ELEMENTS.md`, `CONTRIBUTING.md` y ADRs 0001–0006.
- Ejemplo semilla `examples/traffic_light.slprj` y `examples/README.md`.
- `softladder-cli` ejecuta un proyecto en modo headless (esqueleto funcional del ciclo de scan).

**Siguiente paso inmediato**: M4 — SFC/Grafcet (modelo, motor y editor) sobre el mismo corpus
secuencial, más el cierre de las divergencias de comportamiento listadas en
`testdata/known-divergences.md`.

### Deuda resuelta en M3

- Importador y exportador reales de ClassicLadder, capa por capa, con informe de diagnósticos
  localizados (`SL-E030`, `SL-W030`–`SL-W033`) y passthrough byte a byte de lo no modelado.
- Mapeo exacto de la columna de los bloques (la referencia lee el enable en la columna del *cuerpo*),
  con los enlaces verticales del cuerpo materializados: sin esto los programas importados no
  arrancaban.
- Biblioteca de funciones de expresión (`ABS/MIN/MAX/AVG/POW/SHL/SHR/ROL/ROR` y alias de
  ClassicLadder), literales hexadecimales `$8000` y operadores `&`/`|`.
- Dos fallos del motor corregidos: la base de un minuto valía 60 minutos, y reconfigurar un
  temporizador con el mismo preset reiniciaba un retardo TOF en curso.
- Registro de divergencias de comportamiento: `testdata/known-divergences.md`.

### Deuda resuelta en M2

- Editor con edición real: paleta, colocación con un solo paso de undo, arrastre, borrado, enlace
  vertical, edición de variable con validación y de `params`.
- Undo/redo ilimitado por comandos (`softladder-edit`), probado con `proptest`: deshacer todo
  restaura el proyecto exactamente.
- Banco de simulación persistido en el proyecto, con posiciones de operador en estado de runtime.
- Panel de problemas con `lint` + validación del banco + errores del último scan.
- Fix del motor: enlaces verticales por columna y cables implícitos que propagan la fusión (una rama
  paralela ya no puede saltarse un contacto de stop en serie).

### Deuda resuelta en M1

- Power flow real por columna con propagación vertical y ramas paralelas.
- Entradas dedicadas de los function blocks (reset/preset/up/down de contadores; reset/in/out de
  registros) y registros FIFO/LIFO implementados.
- `CoilJump` y `CoilCall` ejecutados, con guardas de bucle infinito y de profundidad de pila.
- Símbolos enlazados a variables (`Symbol::var`).
- Cadena de migraciones del formato con fixtures `insta` (v1 → v2).
- Tiempo simulado determinista en `run --cycles`.

### Deuda declarada que sigue abierta

- **Stubs por hito**: import/export ClassicLadder (M3), motor SFC (M4, hoy emite `SL-W002`),
  drivers Modbus/GPIO/HAL (M5/M7), servidor del monitor (M6) y Abrir/Guardar en la UI (M2).
- **Símbolos en la UI**: el modelo ya los enlaza a variables (`Symbol::var`), pero el editor todavía
  no los usa para mostrar nombres en el canvas (M2).
- **Sistema de palabras de sistema (`%SW`)**: `%S` existe como bit; falta la familia de palabras de
  sistema (M2, lo reportará el importador de M3 como aviso).

Verificado en M1: `cargo fmt --all --check` y `cargo clippy --workspace --all-targets -- -D warnings`
en verde, `cargo test --workspace` con 167 tests, `run` reproducible (mismo proyecto ⇒ mismo JSON) y
CI en verde en Linux, macOS y Windows.
