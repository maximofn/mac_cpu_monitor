# Consejos para portar `ram_monitor`, `disk_monitor`, `gpu_monitor` a macOS

Notas tomadas mientras montaba `mac_cpu_monitor`. La idea: cuando me pidas hacer
`mac_ram_monitor` / `mac_disk_monitor` / `mac_gpu_monitor`, abrir este archivo y
seguir la receta — no reinventarla. Cada uno es un Cargo workspace + Swift
Package que sigue exactamente la misma forma que `mac_cpu_monitor`.

---

## 1. Investigación previa, sin atajos

Antes de empezar a escribir código, **leer estos cuatro ficheros del sibling Linux**:

```
../<X>_monitor/crates/<X>-monitor-core/src/model.rs   # schema canónico
../<X>_monitor/crates/<X>-monitor-core/src/lib.rs     # DEFAULT_PORT y constantes
../<X>_monitor/crates/<X>-monitord/src/proc_source.rs # cómo se obtiene la métrica en Linux
../<X>_monitor/README.md                              # endpoints y banderas oficiales
```

El `Snapshot` y los structs anidados son **el contrato wire**. Mac y Linux deben
serializar exactamente los mismos campos con los mismos nombres `snake_case`,
o los frontends compartidos (Home Assistant, este Swift tray) silenciosamente
pierden datos.

Después leer el sibling Mac existente como plantilla:

```
../mac_cpu_monitor/                                   # estructura, scripts, naming
```

## 2. Asignación de puertos (ya fija)

Linux usa la banda **9123-9126**, Mac usa la banda **9133-9136** con el mismo
dígito final. Convención:

| Recurso | Linux | Mac  |
|---------|-------|------|
| GPU     | 9123  | 9133 |
| CPU     | 9124  | 9134 |
| RAM     | 9125  | 9135 |
| Disk    | 9126  | 9136 |

Esto permite que un Mac corra sus 4 backends locales **y** túneles SSH a los 4
backends Linux a la vez sin chocar. **Nunca** copiar el puerto Linux directo
(yo lo hice y RAM Linux 9125 chocó con CPU Mac 9125 — fix retroactivo).

## 3. Qué fuente de datos usar para cada métrica

### `mac_ram_monitor`
- **Todo se saca con `sysinfo`** (no hace falta `macmon` para RAM básica):
  `total_memory()`, `used_memory()`, `available_memory()`, `total_swap()`,
  `used_swap()`. Activar `features = ["system"]`.
- Si quieres también potencia del bus de RAM (raro), `macmon::Metrics::ram_power`
  + `Metrics::memory.{ram_usage,swap_usage}` lo expone gratis. Útil sólo si el
  schema Linux tiene un campo equivalente — si no, no lo añadas para no romper
  el contrato.
- Top procesos por uso de memoria: ordenar `sys.processes()` por `memory()`
  descendente, mismo patrón que CPU.

### `mac_disk_monitor`
- `sysinfo` con `features = ["disk"]` da `Disks::new_with_refreshed_list()`,
  `disk.mount_point()`, `total_space()`, `available_space()`, `kind()`,
  `file_system()`. **Sin** I/O por segundo (sysinfo no expone IOPS en macOS).
- Para read/write throughput por disco hace falta IOKit (`IOServiceMatching("IOMedia")`)
  + `getfsstat`. Si el sibling Linux expone `read_bytes_per_s` / `write_bytes_per_s`,
  hay que escribir un wrapper FFI — empezar leyendo `proc_source.rs` Linux para
  ver exactamente qué expone, y solo entonces decidir si abordar IOKit o
  reportar `null` en Mac (mismo patrón que la temperatura en Intel).
- No usar `df` ni `iostat` por subprocess: introduce latencia y fragilidad.

### `mac_gpu_monitor`
- **`macmon` ya tiene casi todo**: `Metrics::gpu_usage` (freq, % from max),
  `temp.gpu_temp_avg`, `gpu_power`. Apple Silicon no tiene per-engine usage
  (no equivalente a `nvidia-smi --query-gpu=utilization.gpu`); reportar el
  cluster único.
- VRAM en Apple Silicon es unified memory: el sibling Linux probablemente
  reporta `memory_total/memory_used` por GPU. En Mac podemos reportar
  `total_memory` (compartida con CPU) o `null` con un comentario claro.
- Si el sibling Linux soporta múltiples GPUs (vector), envolver siempre en
  vector también en Mac aunque tenga un solo elemento, para que el frontend
  común no necesite ramas.

### Temperatura general (cuando aplique)
- Apple Silicon: `macmon` (sin sudo, IOReport).
- Intel Mac: `macsmc` bajo `cfg(target_arch = "x86_64")`. No hay crate único
  que cubra ambos limpiamente — los target deps son la herramienta correcta.

## 4. Trampas técnicas ya descubiertas

### macmon `Sampler` no es `Send`
Tiene un raw `*const __CFDictionary` dentro de `IOHIDSensors`. **Inicializarlo
dentro del closure del std::thread** que samplea, no en `main`. Nada de
`Arc<Mutex<Sampler>>`. Patrón exacto en
`mac_cpu_monitor/crates/mac-cpu-monitord/src/sampler.rs::spawn_sampler`.

### macmon `get_metrics(duration_ms)` bloquea
Reparte la duración en 4 sub-muestras IOReport. `get_metrics(1000)` bloquea ~1 s.
Por eso el sampler vive en un `std::thread` propio, **no en `tokio::spawn`**.
La cadencia de muestreo = duración del macmon ≈ tu intervalo de sample.

### sysinfo necesita ventana entre dos `refresh_cpu_usage`
Si llamas `refresh_cpu_usage` dos veces seguidas sin sleep, los deltas son cero
y todos los porcentajes salen 0. La función `MacCpuSource::sample` mete un
`thread::sleep(sample_window)` entre los dos refreshes — replicar para cualquier
métrica con counters tipo "delta entre dos lecturas".

### sysinfo 0.39 features
Default features (`component`, `disk`, `network`, `system`, `user`) son
demasiadas. Recortar a lo mínimo:
- RAM monitor → `["system"]`
- Disk monitor → `["disk", "system"]` (system para el host_name / kernel_version)
- GPU monitor → `["system"]` (la GPU sale de macmon)

### Versión de Rust
`macmon` usa `edition = "2024"` → necesita rustc ≥ 1.85.
`sysinfo 0.39` necesita rustc ≥ 1.95.
El `rust-toolchain.toml` con `channel = "stable"` resuelve ambos.

### Swift Package
- `Package.swift` con `platforms: [.macOS(.v13)]` y `resources: [.process("Resources")]`.
- En `build-app.sh`, copiar el bundle generado por SwiftPM
  (`<Target>_<Target>.bundle`) dentro de `.app/Contents/MacOS/` o `Bundle.module`
  no resuelve en runtime y el icono base sale `nil`.
- `Info.plist` con `LSUIElement=true` para app menubar-only.
- `Foundation.AsyncBytes.lines` colapsa líneas vacías → no se pueden detectar
  los separadores SSE. El cliente parsea SSE a mano (ver `Client.swift`).
  Asume que el daemon emite **una snapshot completa por evento**, lo cual
  exige que el JSON serializado quepa en una sola línea — `axum::sse::Event::default().json_data(&snap)` lo garantiza.

### NSStatusItem
- No hacer KVO sobre `effectiveAppearance` del button del status item: bucle
  de feedback con el repaint. Usar `DistributedNotificationCenter` y el evento
  `AppleInterfaceThemeChangedNotification`.
- Dedupe de renders por `pct:temp|connected|appearance` — a 1 Hz casi todos
  los ticks tienen estado visible idéntico, repintar es caro.

## 5. Receta de creación de un sibling Mac

```
mac_<X>_monitor/
├── .gitignore                 # copiar de mac_cpu_monitor (target/, .build/, build/, Package.resolved, .DS_Store)
├── Cargo.toml                 # workspace con resolver=2 y workspace.dependencies idénticos
├── rust-toolchain.toml        # channel = "stable"
├── LICENSE                    # MIT, copiar
├── README.md                  # mismo esqueleto que mac_cpu_monitor/README.md
├── CLAUDE.md                  # adaptar el de mac_cpu_monitor (paths, métricas, port)
├── consejos.md                # SOLO copiar este fichero si añade valor; si no, borrar
├── assets/                    # icono base PNG; si pesa >1 MB redimensionar a 512x512 con `sips`
├── crates/
│   ├── mac-<X>-monitor-core/  # solo serde types, sin lógica
│   └── mac-<X>-monitord/      # daemon: sysinfo + (macmon si aplica) + axum
└── front-mac/
    ├── Package.swift          # name = "Mac<X>MonitorTray"
    ├── Info.plist             # CFBundleIdentifier = com.maximofn.mac-<X>-monitor
    ├── Sources/Mac<X>MonitorTray/{main, AppDelegate, Config, Client, Models, StatusBarController, IconRenderer}.swift
    └── scripts/{build-app.sh, install-daemon.sh, install-launchagent.sh, com.maximofn.mac-<X>-monitor*.plist}
```

Pasos en orden:

1. **Leer el sibling Linux completo** (model.rs, proc_source.rs, README, lib.rs).
2. **Crear el workspace** copiando `mac_cpu_monitor/Cargo.toml` y ajustando members.
3. **`mac-<X>-monitor-core/src/model.rs`** — copiar **literal** el `Snapshot`,
   `<X>` y structs anidados desde el sibling Linux. Cambiar solo el `mod.rs`
   path. Mantener el test JSON-roundtrip.
4. **`source.rs`** — escribir `Mac<X>Source::sample()`. Si la métrica es
   `sysinfo`-only no hace falta el `MacmonAdapter`; si es macmon-only o
   mixed, reusar el patrón de `MacCpuSource` + `MacmonAdapter`.
5. **`sampler.rs`, `http/`, `main.rs`, `config.rs`** — copiar de
   `mac_cpu_monitor`, renombrar tipos, ajustar las rutas REST al sibling
   Linux (no inventarlas).
6. **Frontend Swift** — copiar la carpeta `Sources/MacCPUMonitorTray/` entera,
   renombrar y sustituir `CPU` por `<X>` en types/labels. `IconRenderer.swift`
   adaptarse al schema (qué número va en el donut, qué número va en la
   etiqueta lateral).
7. **`scripts/`** — copiar y renombrar los plists / scripts. Hardcodear el
   path absoluto del .app y del binario release (sí, frágil; los scripts ya
   verifican existencia y abortan si falta).
8. **Smoke test**:
   ```
   cargo build --release --workspace
   cd front-mac && ./scripts/build-app.sh
   ../target/release/mac-<X>-monitord --port 913X &
   curl -s http://127.0.0.1:913X/v1/snapshot | jq    # verificar TODOS los campos no-null donde aplique
   "build/Mac <X> Monitor.app/Contents/MacOS/mac-<X>-monitor-tray" --dump-icon /tmp/x.png
   ```
9. **Instalar autostart**: `./scripts/install-daemon.sh && ./scripts/install-launchagent.sh`.
10. **Subir a GitHub**: `gh repo create maximofn/mac_<X>_monitor --public --source . --push`.
    Recordar: el env `GITHUB_TOKEN` que viene en esta sesión **no tiene scope
    `repo:create`**. Hacer `unset GITHUB_TOKEN` antes para que `gh` use el
    token del keyring (el `gho_*`).

## 6. Mini-checklist de "ya lo hice antes y se me olvidó"

- [ ] El puerto que elegí no choca con el equivalente Linux.
- [ ] Cada campo nuevo en `model.rs` tiene su `CodingKeys` en `Models.swift`.
- [ ] El sampler bloqueante vive en `std::thread`, no en `tokio::spawn`.
- [ ] El bundle SwiftPM se copia dentro de `.app/Contents/MacOS/`.
- [ ] El `LSUIElement=true` está en `Info.plist`.
- [ ] El `--dump-icon` produce un PNG válido (~65×44 px en 1× / 130×88 en 2×).
- [ ] `curl /v1/snapshot` devuelve campos reales, no ceros.
- [ ] `gh auth status` muestra el token correcto antes de `repo create`.
- [ ] `rustc --version ≥ 1.95` (sysinfo 0.39).
- [ ] El `consejos.md` se actualiza si descubro una trampa nueva.

## 7. Cosas que NO repetir

- Empezar a escribir Rust antes de leer el sibling Linux. Pierdes una hora
  reinventando un schema que ya existía.
- Elegir el puerto Mac copiando el de Linux. Esa fue **la** colisión que
  obligó a regenerar plists y reinstalar agents.
- Intentar compartir `macmon::Sampler` entre threads. **No compila** y la
  primera vez que lo intenté me costó 10 min de errores de borrowck antes de
  recordarlo.
- Asumir que `sysinfo::System::load_average()` devuelve `Result`. En 0.39 es
  `LoadAvg` directo; en versiones posteriores cambió. Mirar la fuente
  instalada en `~/.cargo/registry/src/index.crates.io-*/sysinfo-<ver>/src/`
  cuando haya duda, no docs.rs (que muestra HEAD).
- Olvidar `swift build --show-bin-path` para localizar el binario de
  SwiftPM — está en `.build/<arch>-apple-macosx/release/`, no en `build/`.
