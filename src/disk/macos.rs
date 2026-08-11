use std::time::Duration;

use objc2_foundation::{
    ns_string, NSNumber, NSURLResourceKey, NSURLVolumeAvailableCapacityForImportantUsageKey,
    NSURLVolumeTotalCapacityKey, NSURL,
};

use super::{DiskObservation, InvalidDiskObservation};

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn mach_continuous_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
}

pub struct ContinuousClock {
    numer: u64,
    denom: u64,
}

impl ContinuousClock {
    pub fn new() -> Result<Self, ContinuousClockError> {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid writable timebase structure for the duration of the call.
        let status = unsafe { mach_timebase_info(&mut info) };
        if status != 0 || info.denom == 0 {
            return Err(ContinuousClockError(status));
        }
        Ok(Self {
            numer: u64::from(info.numer),
            denom: u64::from(info.denom),
        })
    }

    pub fn now(&self) -> Duration {
        // SAFETY: `mach_continuous_time` takes no arguments and is available on the supported
        // macOS deployment target. Unlike `mach_absolute_time`, it advances through system sleep.
        let ticks = unsafe { mach_continuous_time() };
        let nanos = u128::from(ticks) * u128::from(self.numer) / u128::from(self.denom);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContinuousClockError(i32);

pub struct StartupVolumeSampler {
    root: objc2::rc::Retained<NSURL>,
}

impl StartupVolumeSampler {
    pub fn new() -> Self {
        Self {
            root: NSURL::fileURLWithPath(ns_string!("/")),
        }
    }

    pub fn sample(&self, observed_at: Duration) -> Result<DiskObservation, DiskSampleError> {
        let total_bytes = self.resource_bytes(unsafe { NSURLVolumeTotalCapacityKey }, "total")?;
        let available_bytes = self.resource_bytes(
            unsafe { NSURLVolumeAvailableCapacityForImportantUsageKey },
            "available for important usage",
        )?;
        DiskObservation::new(total_bytes, available_bytes, observed_at)
            .map_err(DiskSampleError::InvalidObservation)
    }

    fn resource_bytes(
        &self,
        key: &NSURLResourceKey,
        field: &'static str,
    ) -> Result<u64, DiskSampleError> {
        let mut value = None;
        unsafe { self.root.getResourceValue_forKey_error(&mut value, key) }
            .map_err(|_| DiskSampleError::Unavailable(field))?;
        let number = value
            .ok_or(DiskSampleError::Unavailable(field))?
            .downcast::<NSNumber>()
            .map_err(|_| DiskSampleError::UnexpectedValue(field))?;
        u64::try_from(number.longLongValue()).map_err(|_| DiskSampleError::UnexpectedValue(field))
    }
}

impl Default for StartupVolumeSampler {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiskSampleError {
    Unavailable(&'static str),
    UnexpectedValue(&'static str),
    InvalidObservation(InvalidDiskObservation),
}
