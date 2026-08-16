//! Linux X11 原生能力探测与操作。
//!
//! 本模块是 `floatile-platform` 内部实现；上层不得接触 XID、Atom 或 X11 连接。

use crate::PlatformError;
use crate::capability::{
    CapabilityState, CapabilityUnavailableReason, PlatformCapabilities, PlatformKind,
};
use crate::monitor::{MonitorInfo, MonitorKeySource};
use floatile_core::{MonitorKey, PhysicalPosition, PhysicalSize};
use x11rb::connection::Connection;
use x11rb::protocol::randr::{Connection as RandrConnection, ConnectionExt as _, SetConfig};
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{Atom, AtomEnum, ClipOrdering, ConnectionExt as _, Window};

fn probe_state(
    result: Result<bool, String>,
    unavailable: CapabilityUnavailableReason,
    capability: &'static str,
) -> CapabilityState {
    match result {
        Ok(true) => CapabilityState::Available,
        Ok(false) => CapabilityState::unavailable(unavailable),
        Err(error) => {
            tracing::warn!(capability, %error, "X11 capability probe failed");
            CapabilityState::unavailable(CapabilityUnavailableReason::ProbeFailed)
        }
    }
}

fn compositor_available<C: Connection>(
    connection: &C,
    screen_number: usize,
) -> Result<bool, String> {
    let selection_name = format!("_NET_WM_CM_S{screen_number}");
    let atom = connection
        .intern_atom(true, selection_name.as_bytes())
        .map_err(|error| format!("intern compositor selection: {error}"))?
        .reply()
        .map_err(|error| format!("read compositor selection atom: {error}"))?
        .atom;
    if atom == x11rb::NONE {
        return Ok(false);
    }

    let owner = connection
        .get_selection_owner(atom)
        .map_err(|error| format!("request compositor selection owner: {error}"))?
        .reply()
        .map_err(|error| format!("read compositor selection owner: {error}"))?
        .owner;
    Ok(owner != x11rb::NONE)
}

fn extension_available<C: Connection>(
    connection: &C,
    extension: &'static [u8],
) -> Result<bool, String> {
    connection
        .query_extension(extension)
        .map_err(|error| format!("query extension: {error}"))?
        .reply()
        .map(|reply| reply.present)
        .map_err(|error| format!("read extension query: {error}"))
}

fn ewmh_supports<C: Connection>(
    connection: &C,
    root: Window,
    capability_name: &'static [u8],
) -> Result<bool, String> {
    let supported = connection
        .intern_atom(true, b"_NET_SUPPORTED")
        .map_err(|error| format!("intern _NET_SUPPORTED: {error}"))?
        .reply()
        .map_err(|error| format!("read _NET_SUPPORTED atom: {error}"))?
        .atom;
    let capability = connection
        .intern_atom(true, capability_name)
        .map_err(|error| format!("intern EWMH capability: {error}"))?
        .reply()
        .map_err(|error| format!("read EWMH capability atom: {error}"))?
        .atom;
    if supported == x11rb::NONE || capability == x11rb::NONE {
        return Ok(false);
    }

    let property = connection
        .get_property(false, root, supported, AtomEnum::ATOM, 0, u32::MAX)
        .map_err(|error| format!("request _NET_SUPPORTED: {error}"))?
        .reply()
        .map_err(|error| format!("read _NET_SUPPORTED: {error}"))?;
    let Some(atoms) = property.value32() else {
        return Ok(false);
    };
    Ok(atoms.into_iter().any(|atom| atom == capability))
}

pub(crate) fn probe_capabilities() -> PlatformCapabilities {
    let Ok((connection, screen_number)) = x11rb::connect(None) else {
        return PlatformCapabilities {
            kind: PlatformKind::X11,
            compositing: CapabilityState::unavailable(
                CapabilityUnavailableReason::DisplayUnavailable,
            ),
            click_through: CapabilityState::unavailable(
                CapabilityUnavailableReason::DisplayUnavailable,
            ),
            always_on_top: CapabilityState::unavailable(
                CapabilityUnavailableReason::DisplayUnavailable,
            ),
        };
    };
    let Some(screen) = connection.setup().roots.get(screen_number) else {
        return PlatformCapabilities {
            kind: PlatformKind::X11,
            compositing: CapabilityState::unavailable(CapabilityUnavailableReason::ProbeFailed),
            click_through: CapabilityState::unavailable(CapabilityUnavailableReason::ProbeFailed),
            always_on_top: CapabilityState::unavailable(CapabilityUnavailableReason::ProbeFailed),
        };
    };

    PlatformCapabilities {
        kind: PlatformKind::X11,
        compositing: probe_state(
            compositor_available(&connection, screen_number),
            CapabilityUnavailableReason::CompositorNotDetected,
            "compositing",
        ),
        click_through: probe_state(
            extension_available(&connection, b"SHAPE"),
            CapabilityUnavailableReason::ExtensionUnavailable,
            "click_through",
        ),
        always_on_top: probe_state(
            ewmh_supports(&connection, screen.root, b"_NET_WM_STATE_ABOVE"),
            CapabilityUnavailableReason::WindowManagerUnsupported,
            "always_on_top",
        ),
    }
}

pub(crate) fn set_click_through(window: Window, enabled: bool) -> Result<(), PlatformError> {
    let (connection, _) = x11rb::connect(None)
        .map_err(|error| PlatformError::Platform(format!("connect to X11: {error}")))?;
    let shape = connection
        .query_extension(b"SHAPE")
        .map_err(|error| PlatformError::Platform(format!("query SHAPE extension: {error}")))?
        .reply()
        .map_err(|error| PlatformError::Platform(format!("read SHAPE extension: {error}")))?;
    if !shape.present {
        return Err(PlatformError::Unsupported(
            "X11 server does not provide the SHAPE extension",
        ));
    }

    if enabled {
        connection
            .shape_rectangles(
                SO::SET,
                SK::INPUT,
                ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &[],
            )
            .map_err(|error| {
                PlatformError::Platform(format!("set empty XShape input region: {error}"))
            })?
            .check()
            .map_err(|error| {
                PlatformError::Platform(format!("apply empty XShape input region: {error}"))
            })?;
    } else {
        connection
            .shape_mask(SO::SET, SK::INPUT, window, 0, 0, x11rb::NONE)
            .map_err(|error| {
                PlatformError::Platform(format!("reset XShape input region: {error}"))
            })?
            .check()
            .map_err(|error| {
                PlatformError::Platform(format!("apply reset XShape input region: {error}"))
            })?;
    }
    connection
        .flush()
        .map_err(|error| PlatformError::Platform(format!("flush XShape request: {error}")))
}

fn edid_fingerprint(edid: &[u8]) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let hash = edid.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    });
    format!("edid-{hash:016x}")
}

fn monitor_key(name: &str, edid: Option<&[u8]>) -> (MonitorKey, MonitorKeySource) {
    match edid {
        Some(edid) if !edid.is_empty() => {
            (MonitorKey(edid_fingerprint(edid)), MonitorKeySource::Edid)
        }
        _ => (
            MonitorKey(format!("x11-output-{name}")),
            MonitorKeySource::ConnectorName,
        ),
    }
}

fn output_edid<C: Connection>(
    connection: &C,
    output: u32,
    edid_atom: Atom,
) -> Result<Option<Vec<u8>>, PlatformError> {
    if edid_atom == x11rb::NONE {
        return Ok(None);
    }
    let property = connection
        .randr_get_output_property(output, edid_atom, AtomEnum::ANY, 0, 256, false, false)
        .map_err(|error| PlatformError::Platform(format!("request RandR EDID property: {error}")))?
        .reply()
        .map_err(|error| PlatformError::Platform(format!("read RandR EDID property: {error}")))?;
    if property.bytes_after != 0 {
        return Err(PlatformError::Platform(
            "RandR EDID property exceeded the bounded read".into(),
        ));
    }
    if property.format == 8 && !property.data.is_empty() {
        Ok(Some(property.data))
    } else {
        Ok(None)
    }
}

pub(crate) fn enumerate_monitors() -> Result<Vec<MonitorInfo>, PlatformError> {
    let (connection, screen_number) = x11rb::connect(None)
        .map_err(|error| PlatformError::Platform(format!("connect to X11: {error}")))?;
    let screen = connection
        .setup()
        .roots
        .get(screen_number)
        .ok_or_else(|| PlatformError::Platform("X11 screen index is invalid".into()))?;
    let version = connection
        .randr_query_version(1, 3)
        .map_err(|error| PlatformError::Platform(format!("query RandR version: {error}")))?
        .reply()
        .map_err(|error| PlatformError::Platform(format!("read RandR version: {error}")))?;
    if (version.major_version, version.minor_version) < (1, 3) {
        return Err(PlatformError::Unsupported(
            "X11 monitor enumeration requires RandR 1.3",
        ));
    }

    let resources = connection
        .randr_get_screen_resources_current(screen.root)
        .map_err(|error| PlatformError::Platform(format!("request RandR resources: {error}")))?
        .reply()
        .map_err(|error| PlatformError::Platform(format!("read RandR resources: {error}")))?;
    let primary_output = connection
        .randr_get_output_primary(screen.root)
        .map_err(|error| PlatformError::Platform(format!("request RandR primary output: {error}")))?
        .reply()
        .map_err(|error| PlatformError::Platform(format!("read RandR primary output: {error}")))?
        .output;
    let edid_atom = connection
        .intern_atom(true, b"EDID")
        .map_err(|error| PlatformError::Platform(format!("intern EDID atom: {error}")))?
        .reply()
        .map_err(|error| PlatformError::Platform(format!("read EDID atom: {error}")))?
        .atom;

    let mut monitors = Vec::new();
    for output in resources.outputs {
        let output_info = connection
            .randr_get_output_info(output, resources.config_timestamp)
            .map_err(|error| PlatformError::Platform(format!("request RandR output: {error}")))?
            .reply()
            .map_err(|error| PlatformError::Platform(format!("read RandR output: {error}")))?;
        if output_info.status != SetConfig::SUCCESS {
            return Err(PlatformError::Platform(format!(
                "RandR output query returned status {:?}",
                output_info.status
            )));
        }
        if output_info.connection != RandrConnection::CONNECTED || output_info.crtc == x11rb::NONE {
            continue;
        }
        let crtc = connection
            .randr_get_crtc_info(output_info.crtc, resources.config_timestamp)
            .map_err(|error| PlatformError::Platform(format!("request RandR CRTC: {error}")))?
            .reply()
            .map_err(|error| PlatformError::Platform(format!("read RandR CRTC: {error}")))?;
        if crtc.status != SetConfig::SUCCESS {
            return Err(PlatformError::Platform(format!(
                "RandR CRTC query returned status {:?}",
                crtc.status
            )));
        }
        if crtc.width == 0 || crtc.height == 0 {
            continue;
        }

        let name = if output_info.name.is_empty() {
            format!("output-{output}")
        } else {
            String::from_utf8_lossy(&output_info.name).into_owned()
        };
        let edid = output_edid(&connection, output, edid_atom)?;
        let (key, key_source) = monitor_key(&name, edid.as_deref());
        let physical_size_mm =
            (output_info.mm_width > 0 && output_info.mm_height > 0).then_some(PhysicalSize {
                width: output_info.mm_width,
                height: output_info.mm_height,
            });
        monitors.push(MonitorInfo {
            key,
            key_source,
            name,
            position: PhysicalPosition {
                x: i32::from(crtc.x),
                y: i32::from(crtc.y),
            },
            size: PhysicalSize {
                width: u32::from(crtc.width),
                height: u32::from(crtc.height),
            },
            physical_size_mm,
            primary: output == primary_output,
        });
    }

    if monitors.is_empty() {
        return Err(PlatformError::Platform(
            "RandR reported no active outputs".into(),
        ));
    }
    if !monitors.iter().any(|monitor| monitor.primary) {
        monitors[0].primary = true;
    }
    Ok(monitors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edid_key_is_stable_and_content_sensitive() {
        let first = edid_fingerprint(&[0x00, 0xff, 0xaa, 0x55]);
        assert_eq!(first, edid_fingerprint(&[0x00, 0xff, 0xaa, 0x55]));
        assert_ne!(first, edid_fingerprint(&[0x00, 0xff, 0xaa, 0x54]));
        assert!(first.starts_with("edid-"));
    }

    #[test]
    fn missing_edid_falls_back_to_connector_name() {
        let (key, source) = monitor_key("DP-1", None);
        assert_eq!(key.as_str(), "x11-output-DP-1");
        assert_eq!(source, MonitorKeySource::ConnectorName);
    }
}
