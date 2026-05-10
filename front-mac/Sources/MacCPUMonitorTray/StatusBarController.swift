import AppKit
import Foundation

private let repoURL = URL(string: "https://github.com/maximofn/mac_cpu_monitor")!
private let coffeeURL = URL(string: "https://www.buymeacoffee.com/maximofn")!

enum TrayState: Sendable {
    case connecting
    case connected(Snapshot)
    case disconnected(String)
}

@MainActor
final class StatusBarController: NSObject {
    private let statusItem: NSStatusItem
    private let renderer: IconRenderer
    private let backendURL: String
    private var state: TrayState = .connecting
    private var lastAppearance: IconAppearance = .dark
    private var lastRenderedKey: String = ""

    init(renderer: IconRenderer, backendURL: String) {
        self.renderer = renderer
        self.backendURL = backendURL
        self.statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        super.init()
        if let button = statusItem.button {
            button.imagePosition = .imageLeft
            button.toolTip = "Mac CPU Monitor — connecting to \(backendURL)"
        }
        // System-wide light/dark toggle. Don't KVO `effectiveAppearance` on the
        // button — AppKit re-evaluates that during repaints and any reaction
        // there feeds back into refreshIcon → set image → repaint → KVO loop.
        DistributedNotificationCenter.default.addObserver(
            self,
            selector: #selector(appearanceChanged),
            name: Notification.Name("AppleInterfaceThemeChangedNotification"),
            object: nil
        )
        lastAppearance = currentAppearance
        applyState(.connecting)
    }

    deinit {
        DistributedNotificationCenter.default.removeObserver(self)
    }

    @objc private func appearanceChanged() {
        Task { @MainActor in
            self.lastAppearance = self.currentAppearance
            self.lastRenderedKey = ""
            self.refreshIcon()
        }
    }

    func applyState(_ new: TrayState) {
        state = new
        refreshIcon()
        refreshMenu()
        refreshTooltip()
    }

    private var currentAppearance: IconAppearance {
        let appearance = statusItem.button?.effectiveAppearance ?? NSApp.effectiveAppearance
        let match = appearance.bestMatch(from: [.darkAqua, .vibrantDark, .aqua, .vibrantLight])
        switch match {
        case .darkAqua, .vibrantDark: return .dark
        default: return .light
        }
    }

    private func refreshIcon() {
        let (cpu, connected): (CPU?, Bool) = {
            switch state {
            case .connected(let snap): return (snap.cpu, true)
            default: return (nil, false)
            }
        }()
        // Dedupe identical renders — at 1 Hz most ticks have identical visible state.
        let key = renderKey(cpu: cpu, connected: connected, appearance: lastAppearance)
        if key == lastRenderedKey { return }
        lastRenderedKey = key
        if let img = renderer.renderImage(cpu: cpu, connected: connected, appearance: lastAppearance) {
            statusItem.button?.image = img
        }
    }

    private func renderKey(cpu: CPU?, connected: Bool, appearance: IconAppearance) -> String {
        var parts: [String] = ["\(connected)", "\(appearance)"]
        if let c = cpu {
            let pct = Int(c.usagePercent.rounded())
            let temp = c.temperatureC.map { Int($0.rounded()) } ?? -999
            parts.append("\(pct):\(temp)")
        }
        return parts.joined(separator: "|")
    }

    private func refreshTooltip() {
        guard let button = statusItem.button else { return }
        switch state {
        case .connecting:
            button.toolTip = "Mac CPU Monitor — connecting to \(backendURL)"
        case .connected(let snap):
            let c = snap.cpu
            let model = c.model ?? "CPU"
            let tempStr = c.temperatureC.map { String(format: "%.0f°C", $0) } ?? "—"
            let header = "\(model) — \(Int(c.usagePercent.rounded()))% (\(tempStr))"
            var lines: [String] = [header]
            if let load = c.loadAverage {
                lines.append(String(format: "Load: %.2f / %.2f / %.2f", load.one, load.five, load.fifteen))
            }
            if let f = c.frequencyMHz {
                lines.append(String(format: "Frequency: %.0f MHz", f))
            }
            lines.append("\(c.logicalCores) logical / \(c.physicalCores.map(String.init) ?? "?") physical cores")
            button.toolTip = lines.joined(separator: "\n")
        case .disconnected(let err):
            button.toolTip = "Backend offline: \(err)"
        }
    }

    private func refreshMenu() {
        let menu = NSMenu()
        menu.autoenablesItems = false

        switch state {
        case .connecting:
            menu.addItem(disabledItem("Connecting to \(backendURL)…"))
            menu.addItem(.separator())
        case .disconnected(let err):
            menu.addItem(disabledItem("Backend offline: \(err)"))
            menu.addItem(disabledItem("Backend: \(backendURL)"))
            menu.addItem(.separator())
        case .connected(let snap):
            let c = snap.cpu
            let header = c.model ?? "CPU"
            let item = NSMenuItem(title: header, action: nil, keyEquivalent: "")
            item.submenu = cpuSubmenu(for: c)
            menu.addItem(item)

            menu.addItem(.separator())
            menu.addItem(disabledItem("Backend: \(backendURL)"))
            if let kernel = snap.kernel {
                menu.addItem(disabledItem("Kernel: \(kernel)"))
            }
            menu.addItem(disabledItem("Updated: \(shortTime(snap.timestamp))"))
            menu.addItem(.separator())
        }

        let repo = NSMenuItem(title: "Repository", action: #selector(openRepo), keyEquivalent: "")
        repo.target = self
        menu.addItem(repo)
        let coffee = NSMenuItem(title: "Buy me a coffee", action: #selector(openCoffee), keyEquivalent: "")
        coffee.target = self
        menu.addItem(coffee)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit", action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)

        statusItem.menu = menu
    }

    private func cpuSubmenu(for cpu: CPU) -> NSMenu {
        let m = NSMenu()
        m.autoenablesItems = false
        if let v = cpu.vendor { m.addItem(disabledItem("Vendor: \(v)")) }
        m.addItem(disabledItem(String(format: "Usage: %.1f%%", cpu.usagePercent)))
        if let t = cpu.temperatureC {
            let sensor = cpu.primarySensor.map { " (\($0))" } ?? ""
            m.addItem(disabledItem(String(format: "Temperature: %.0f°C%@", t, sensor)))
        }
        if let f = cpu.frequencyMHz {
            m.addItem(disabledItem(String(format: "Frequency: %.0f MHz", f)))
        }
        m.addItem(disabledItem(
            "Cores: \(cpu.logicalCores) logical / \(cpu.physicalCores.map(String.init) ?? "?") physical"
        ))
        if let load = cpu.loadAverage {
            m.addItem(disabledItem(String(format: "Load avg: %.2f, %.2f, %.2f",
                                           load.one, load.five, load.fifteen)))
        }
        if let up = cpu.uptimeS {
            m.addItem(disabledItem("Uptime: \(formatUptime(up))"))
        }

        if !cpu.temperatures.isEmpty {
            m.addItem(.separator())
            m.addItem(disabledItem("Temperature sensors (\(cpu.temperatures.count))"))
            for s in cpu.temperatures {
                m.addItem(disabledItem(String(format: "  %@/%@: %.0f°C", s.chip, s.label, s.tempC)))
            }
        }

        if !cpu.perCoreUsage.isEmpty {
            m.addItem(.separator())
            m.addItem(disabledItem("Per-core usage"))
            for (idx, u) in cpu.perCoreUsage.enumerated() {
                m.addItem(disabledItem(String(format: "  CPU%2d: %5.1f%%", idx, u)))
            }
        }

        m.addItem(.separator())
        if cpu.processes.isEmpty {
            m.addItem(disabledItem("No CPU processes"))
        } else {
            m.addItem(disabledItem("Top processes (\(cpu.processes.count))"))
            for proc in cpu.processes {
                let line = String(
                    format: "  %6d %5.1f%%  %@ (%@)",
                    proc.pid,
                    proc.cpuPercent,
                    proc.name as NSString,
                    formatBytes(proc.memoryBytes) as NSString
                )
                m.addItem(disabledItem(line))
            }
        }
        return m
    }

    @objc private func openRepo() { NSWorkspace.shared.open(repoURL) }
    @objc private func openCoffee() { NSWorkspace.shared.open(coffeeURL) }
    @objc private func quit() { NSApp.terminate(nil) }
}

// MARK: - Helpers

private func disabledItem(_ title: String) -> NSMenuItem {
    let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
    item.isEnabled = false
    return item
}

private func formatBytes(_ bytes: UInt64) -> String {
    let gib: Double = 1024 * 1024 * 1024
    let mib: Double = 1024 * 1024
    let b = Double(bytes)
    if b >= gib { return String(format: "%.2f GiB", b / gib) }
    if b >= mib { return String(format: "%.0f MiB", b / mib) }
    return "\(bytes) B"
}

private func formatUptime(_ seconds: UInt64) -> String {
    let d = seconds / 86_400
    let h = (seconds % 86_400) / 3_600
    let m = (seconds % 3_600) / 60
    if d > 0 { return "\(d)d \(h)h \(m)m" }
    if h > 0 { return "\(h)h \(m)m" }
    return "\(m)m"
}

/// "2026-05-06T10:11:12.345Z" → "10:11:12".
private func shortTime(_ rfc3339: String) -> String {
    guard let tIdx = rfc3339.firstIndex(of: "T") else { return rfc3339 }
    let after = rfc3339[rfc3339.index(after: tIdx)...]
    if let dot = after.firstIndex(of: ".") {
        return String(after[..<dot])
    }
    if let plus = after.firstIndex(where: { $0 == "+" || $0 == "Z" || $0 == "-" }) {
        return String(after[..<plus])
    }
    return String(after)
}
