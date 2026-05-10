use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub timestamp: String,
    pub host: String,
    pub kernel: Option<String>,
    pub cpu: Cpu,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Cpu {
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub logical_cores: u32,
    pub physical_cores: Option<u32>,
    pub usage_percent: f32,
    pub per_core_usage: Vec<f32>,
    pub temperature_c: Option<f32>,
    pub primary_sensor: Option<String>,
    pub temperatures: Vec<TempSensor>,
    pub frequency_mhz: Option<f32>,
    pub load_average: Option<LoadAverage>,
    pub uptime_s: Option<u64>,
    pub processes: Vec<Process>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TempSensor {
    pub chip: String,
    pub label: String,
    pub temp_c: f32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LoadAverage {
    pub one: f32,
    pub five: f32,
    pub fifteen: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_roundtrips_through_json() {
        let snap = Snapshot {
            timestamp: "2026-05-10T12:00:00Z".to_string(),
            host: "mac".to_string(),
            kernel: Some("Darwin 25.3.0".to_string()),
            cpu: Cpu {
                model: Some("Apple M1 Pro".to_string()),
                vendor: Some("Apple".to_string()),
                logical_cores: 10,
                physical_cores: Some(10),
                usage_percent: 12.5,
                per_core_usage: vec![10.0; 10],
                temperature_c: Some(48.0),
                primary_sensor: Some("SOC".to_string()),
                temperatures: vec![TempSensor {
                    chip: "SOC".to_string(),
                    label: "CPU".to_string(),
                    temp_c: 48.0,
                }],
                frequency_mhz: Some(3220.0),
                load_average: Some(LoadAverage {
                    one: 0.5,
                    five: 0.6,
                    fifteen: 0.4,
                }),
                uptime_s: Some(12345),
                processes: vec![Process {
                    pid: 1234,
                    name: "WindowServer".to_string(),
                    cpu_percent: 5.4,
                    memory_bytes: 1_500_000_000,
                }],
            },
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
    }
}
