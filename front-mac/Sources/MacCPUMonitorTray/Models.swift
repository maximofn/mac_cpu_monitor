import Foundation

// Mirror of crates/mac-cpu-monitor-core/src/model.rs. The Rust types are the
// canonical schema (API path /v1/...). If a field is added there, replicate it
// here verbatim or the JSON decode will silently drop data.

struct Snapshot: Codable, Equatable, Sendable {
    let timestamp: String
    let host: String
    let kernel: String?
    let cpu: CPU
}

struct CPU: Codable, Equatable, Sendable {
    let model: String?
    let vendor: String?
    let logicalCores: UInt32
    let physicalCores: UInt32?
    let usagePercent: Float
    let perCoreUsage: [Float]
    let temperatureC: Float?
    let primarySensor: String?
    let temperatures: [TempSensor]
    let frequencyMHz: Float?
    let loadAverage: LoadAverage?
    let uptimeS: UInt64?
    let processes: [CPUProcess]

    enum CodingKeys: String, CodingKey {
        case model
        case vendor
        case logicalCores = "logical_cores"
        case physicalCores = "physical_cores"
        case usagePercent = "usage_percent"
        case perCoreUsage = "per_core_usage"
        case temperatureC = "temperature_c"
        case primarySensor = "primary_sensor"
        case temperatures
        case frequencyMHz = "frequency_mhz"
        case loadAverage = "load_average"
        case uptimeS = "uptime_s"
        case processes
    }
}

struct TempSensor: Codable, Equatable, Sendable {
    let chip: String
    let label: String
    let tempC: Float

    enum CodingKeys: String, CodingKey {
        case chip
        case label
        case tempC = "temp_c"
    }
}

struct LoadAverage: Codable, Equatable, Sendable {
    let one: Float
    let five: Float
    let fifteen: Float
}

struct CPUProcess: Codable, Equatable, Sendable {
    let pid: UInt32
    let name: String
    let cpuPercent: Float
    let memoryBytes: UInt64

    enum CodingKeys: String, CodingKey {
        case pid
        case name
        case cpuPercent = "cpu_percent"
        case memoryBytes = "memory_bytes"
    }
}
