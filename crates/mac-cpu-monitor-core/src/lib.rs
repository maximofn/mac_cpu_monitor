pub mod model;

pub use model::{Cpu, LoadAverage, Process, Snapshot, TempSensor};

pub const DEFAULT_PORT: u16 = 9125;
pub const DEFAULT_BIND: &str = "127.0.0.1";
pub const API_VERSION: &str = "v1";
