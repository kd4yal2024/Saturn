//! Saturn-owned FFTW wisdom generation and import.
//!
//! Wisdom is a derived, machine-local cache. The installed maintenance helper
//! fingerprints the CPU, FFTW package and bridge binary, then invokes the CLI
//! entry points here only when that fingerprint changes.

use std::env;
use std::path::{Path, PathBuf};

#[cfg(not(saturn_bridge_stub_native))]
use std::ffi::CString;

const MIN_WISDOM_SIZE: usize = 64;
pub const DEFAULT_MAX_WISDOM_SIZE: usize = 262_144;
#[cfg(not(saturn_bridge_stub_native))]
const FFTW_FORWARD: i32 = -1;
#[cfg(not(saturn_bridge_stub_native))]
const FFTW_BACKWARD: i32 = 1;
#[cfg(not(saturn_bridge_stub_native))]
const FFTW_PATIENT: u32 = 1 << 5;

#[cfg(not(saturn_bridge_stub_native))]
#[repr(C)]
struct FftwComplex {
    re: f64,
    im: f64,
}

#[cfg(not(saturn_bridge_stub_native))]
mod native {
    use super::FftwComplex;
    use libc::{c_char, c_double, c_int, c_uint, c_void, size_t};

    unsafe extern "C" {
        pub fn fftw_malloc(size: size_t) -> *mut c_void;
        pub fn fftw_free(ptr: *mut c_void);
        pub fn fftw_plan_dft_1d(
            size: c_int,
            input: *mut FftwComplex,
            output: *mut FftwComplex,
            sign: c_int,
            flags: c_uint,
        ) -> *mut c_void;
        pub fn fftw_plan_dft_r2c_1d(
            size: c_int,
            input: *mut c_double,
            output: *mut FftwComplex,
            flags: c_uint,
        ) -> *mut c_void;
        pub fn fftw_plan_dft_c2r_1d(
            size: c_int,
            input: *mut FftwComplex,
            output: *mut c_double,
            flags: c_uint,
        ) -> *mut c_void;
        pub fn fftw_destroy_plan(plan: *mut c_void);
        pub fn fftw_forget_wisdom();
        pub fn fftw_export_wisdom_to_filename(path: *const c_char) -> c_int;
        pub fn fftw_import_wisdom_from_filename(path: *const c_char) -> c_int;
    }
}

#[cfg(not(saturn_bridge_stub_native))]
fn c_path(path: &Path) -> Result<CString, String> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("FFTW wisdom path contains a NUL byte: {}", path.display()))
}

fn validate_max_size(max_size: usize) -> Result<(), String> {
    if !(MIN_WISDOM_SIZE..=DEFAULT_MAX_WISDOM_SIZE).contains(&max_size)
        || !max_size.is_power_of_two()
    {
        return Err(format!(
            "FFTW wisdom maximum must be a power of two between {MIN_WISDOM_SIZE} and {DEFAULT_MAX_WISDOM_SIZE}, got {max_size}"
        ));
    }
    Ok(())
}

fn configured_max_size() -> Result<usize, String> {
    match env::var("SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE") {
        Ok(value) => {
            let parsed = value.trim().parse::<usize>().map_err(|_| {
                format!("invalid SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE value: {value}")
            })?;
            validate_max_size(parsed)?;
            Ok(parsed)
        }
        Err(_) => Ok(DEFAULT_MAX_WISDOM_SIZE),
    }
}

#[cfg(not(saturn_bridge_stub_native))]
struct FftwBuffer(*mut libc::c_void);

#[cfg(not(saturn_bridge_stub_native))]
impl Drop for FftwBuffer {
    fn drop(&mut self) {
        unsafe { native::fftw_free(self.0) };
    }
}

#[cfg(not(saturn_bridge_stub_native))]
struct FftwPlan(*mut libc::c_void);

#[cfg(not(saturn_bridge_stub_native))]
impl Drop for FftwPlan {
    fn drop(&mut self) {
        unsafe { native::fftw_destroy_plan(self.0) };
    }
}

#[cfg(not(saturn_bridge_stub_native))]
fn checked_plan(plan: *mut libc::c_void, description: &str) -> Result<FftwPlan, String> {
    if plan.is_null() {
        Err(format!("FFTW failed to create {description}"))
    } else {
        Ok(FftwPlan(plan))
    }
}

#[cfg(not(saturn_bridge_stub_native))]
pub fn generate(path: &Path, max_size: usize) -> Result<(), String> {
    validate_max_size(max_size)?;
    let c_path = c_path(path)?;
    let complex_bytes = (max_size + 1)
        .checked_mul(std::mem::size_of::<FftwComplex>())
        .ok_or_else(|| "FFTW wisdom allocation size overflow".to_string())?;

    let input = FftwBuffer(unsafe { native::fftw_malloc(complex_bytes) });
    let output = FftwBuffer(unsafe { native::fftw_malloc(complex_bytes) });
    if input.0.is_null() || output.0.is_null() {
        return Err(format!(
            "unable to allocate {} bytes per FFTW wisdom buffer",
            complex_bytes
        ));
    }

    unsafe { native::fftw_forget_wisdom() };
    let mut size = MIN_WISDOM_SIZE;
    while size <= max_size {
        eprintln!("saturn-bridge: planning complex forward FFT size {size}");
        drop(checked_plan(
            unsafe {
                native::fftw_plan_dft_1d(
                    size as i32,
                    input.0.cast(),
                    output.0.cast(),
                    FFTW_FORWARD,
                    FFTW_PATIENT,
                )
            },
            &format!("complex forward plan of size {size}"),
        )?);

        eprintln!("saturn-bridge: planning complex backward FFT size {size}");
        drop(checked_plan(
            unsafe {
                native::fftw_plan_dft_1d(
                    size as i32,
                    input.0.cast(),
                    output.0.cast(),
                    FFTW_BACKWARD,
                    FFTW_PATIENT,
                )
            },
            &format!("complex backward plan of size {size}"),
        )?);

        eprintln!(
            "saturn-bridge: planning complex backward FFT size {}",
            size + 1
        );
        drop(checked_plan(
            unsafe {
                native::fftw_plan_dft_1d(
                    (size + 1) as i32,
                    input.0.cast(),
                    output.0.cast(),
                    FFTW_BACKWARD,
                    FFTW_PATIENT,
                )
            },
            &format!("complex backward plan of size {}", size + 1),
        )?);

        size *= 2;
    }

    size = MIN_WISDOM_SIZE;
    while size <= max_size {
        eprintln!("saturn-bridge: planning real forward FFT size {size}");
        drop(checked_plan(
            unsafe {
                native::fftw_plan_dft_r2c_1d(
                    size as i32,
                    input.0.cast(),
                    output.0.cast(),
                    FFTW_PATIENT,
                )
            },
            &format!("real forward plan of size {size}"),
        )?);

        eprintln!("saturn-bridge: planning real inverse FFT size {size}");
        drop(checked_plan(
            unsafe {
                native::fftw_plan_dft_c2r_1d(
                    size as i32,
                    input.0.cast(),
                    output.0.cast(),
                    FFTW_PATIENT,
                )
            },
            &format!("real inverse plan of size {size}"),
        )?);

        size *= 2;
    }

    if unsafe { native::fftw_export_wisdom_to_filename(c_path.as_ptr()) } != 1 {
        return Err(format!(
            "failed to export FFTW wisdom to {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(saturn_bridge_stub_native)]
pub fn generate(_path: &Path, max_size: usize) -> Result<(), String> {
    validate_max_size(max_size)?;
    Err("FFTW wisdom generation is unavailable in native-stub builds".to_string())
}

#[cfg(not(saturn_bridge_stub_native))]
pub fn import(path: &Path) -> Result<(), String> {
    let c_path = c_path(path)?;
    if unsafe { native::fftw_import_wisdom_from_filename(c_path.as_ptr()) } == 1 {
        Ok(())
    } else {
        Err(format!("FFTW rejected wisdom file {}", path.display()))
    }
}

#[cfg(saturn_bridge_stub_native)]
pub fn import(_path: &Path) -> Result<(), String> {
    Err("FFTW wisdom import is unavailable in native-stub builds".to_string())
}

pub fn import_configured() {
    let Some(path) = env::var_os("SATURN_BRIDGE_FFTW_WISDOM_PATH") else {
        return;
    };
    let path = PathBuf::from(path);
    match import(&path) {
        Ok(()) => eprintln!(
            "saturn-bridge: imported FFTW wisdom from {}",
            path.display()
        ),
        Err(error) => eprintln!(
            "saturn-bridge: FFTW wisdom unavailable ({error}); using runtime FFTW planning"
        ),
    }
}

pub fn handle_cli(args: &[String]) -> Option<Result<(), String>> {
    let command = args.first()?.as_str();
    match command {
        "--generate-fftw-wisdom" => Some((|| {
            if args.len() != 2 {
                return Err("usage: saturn-bridge --generate-fftw-wisdom PATH".to_string());
            }
            let path = Path::new(&args[1]);
            let max_size = configured_max_size()?;
            generate(path, max_size)?;
            eprintln!(
                "saturn-bridge: exported FFTW wisdom through size {max_size} to {}",
                path.display()
            );
            Ok(())
        })()),
        "--validate-fftw-wisdom" => Some((|| {
            if args.len() != 2 {
                return Err("usage: saturn-bridge --validate-fftw-wisdom PATH".to_string());
            }
            let path = Path::new(&args[1]);
            import(path)?;
            eprintln!("saturn-bridge: validated FFTW wisdom at {}", path.display());
            Ok(())
        })()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wisdom_size_requires_supported_power_of_two() {
        assert!(validate_max_size(64).is_ok());
        assert!(validate_max_size(2048).is_ok());
        assert!(validate_max_size(DEFAULT_MAX_WISDOM_SIZE).is_ok());
        assert!(validate_max_size(63).is_err());
        assert!(validate_max_size(1000).is_err());
        assert!(validate_max_size(DEFAULT_MAX_WISDOM_SIZE * 2).is_err());
    }

    #[test]
    fn wisdom_cli_rejects_missing_path() {
        let result = handle_cli(&["--generate-fftw-wisdom".to_string()]);
        assert!(result.expect("command should be recognized").is_err());
    }
}
