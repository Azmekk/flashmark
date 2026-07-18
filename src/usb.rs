//! USB topology walk: drive letter → physical disk → USB device → parent hub,
//! then query the hub for the negotiated link speed and device descriptor.
//!
//! The interesting comparison is `bcdUSB` (what spec the device *claims* to
//! implement, from its own descriptor) against the speed the hub actually
//! *negotiated* for the port. A "USB 3.0" stick running at High-Speed (480
//! Mbps) means the drive, cable, or port is not delivering what the label
//! promises.

use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{self, Error, ErrorKind};
use std::mem::size_of;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DRP_ADDRESS, CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_ID_Size,
    CM_Get_Device_IDW, CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW,
    CM_Get_DevNode_Registry_PropertyW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL,
    CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::DEVPKEY_Device_InstanceId;
use windows::Win32::Devices::Usb::{
    GUID_DEVINTERFACE_USB_HUB, IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION,
    IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2,
    USB_DESCRIPTOR_REQUEST, USB_NODE_CONNECTION_INFORMATION_EX,
    USB_NODE_CONNECTION_INFORMATION_EX_V2,
};
use windows::Win32::Foundation::{DEVPROPKEY, HANDLE};
use windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER,
};
use windows::core::{GUID, PCWSTR};

use crate::drive::{Drive, wide};

const USB_STRING_DESCRIPTOR_TYPE: u8 = 3;
const GENERIC_WRITE: u32 = 0x4000_0000;

#[derive(Serialize, Clone, Copy, PartialEq, Eq, PartialOrd)]
pub enum LinkSpeed {
    Low,
    Full,
    High,
    Super,
    SuperPlus,
}

impl LinkSpeed {
    pub fn label(self) -> &'static str {
        match self {
            LinkSpeed::Low => "1.5 Mbps — USB 1.x Low-Speed",
            LinkSpeed::Full => "12 Mbps — USB 1.x Full-Speed",
            LinkSpeed::High => "480 Mbps — USB 2.0 High-Speed",
            LinkSpeed::Super => "5 Gbps — USB 3 SuperSpeed",
            LinkSpeed::SuperPlus => "10+ Gbps — USB 3 SuperSpeed+",
        }
    }

    /// Practical effective throughput ceiling in MB/s (protocol overhead
    /// included), for sanity-checking measured speeds.
    pub fn effective_ceiling_mbps(self) -> f64 {
        match self {
            LinkSpeed::Low => 0.15,
            LinkSpeed::Full => 1.0,
            LinkSpeed::High => 40.0,
            LinkSpeed::Super => 450.0,
            LinkSpeed::SuperPlus => 900.0,
        }
    }
}

#[derive(Serialize)]
pub struct UsbInfo {
    pub instance_id: String,
    pub vid: u16,
    pub pid: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    /// bcdUSB from the device descriptor — the spec the device claims.
    pub bcd_usb: u16,
    pub negotiated: LinkSpeed,
    pub port: u32,
    pub hub_instance_id: String,
}

impl UsbInfo {
    /// "3.2" style rendering of bcdUSB.
    pub fn claimed_version(&self) -> String {
        format!("{:x}.{:x}", self.bcd_usb >> 8, (self.bcd_usb >> 4) & 0xF)
    }

    pub fn claims_usb3(&self) -> bool {
        self.bcd_usb >= 0x0300
    }

    /// True when the device claims USB 3 but the link is running at USB 2 or
    /// below — the port, cable, or device is not delivering.
    pub fn link_downgraded(&self) -> bool {
        self.claims_usb3() && self.negotiated < LinkSpeed::Super
    }
}

fn cr_err(what: &str, cr: u32) -> Error {
    Error::other(format!("{what} failed (CONFIGRET {cr})"))
}

/// Storage device number for a volume or disk handle.
fn device_number(file: &File) -> io::Result<STORAGE_DEVICE_NUMBER> {
    let mut num = STORAGE_DEVICE_NUMBER::default();
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            HANDLE(file.as_raw_handle()),
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            0,
            Some(&mut num as *mut _ as *mut _),
            size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(|e| Error::other(format!("IOCTL_STORAGE_GET_DEVICE_NUMBER: {e}")))?;
    Ok(num)
}

fn open_query(path: &str) -> io::Result<File> {
    OpenOptions::new().access_mode(0).open(path)
}

/// All present device-interface paths for a class GUID, optionally filtered to
/// one device instance.
fn interface_list(guid: &GUID, device_id: Option<&[u16]>) -> io::Result<Vec<String>> {
    let id_ptr = device_id.map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr()));
    let mut len = 0u32;
    let cr = unsafe {
        CM_Get_Device_Interface_List_SizeW(&mut len, guid, id_ptr, CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
    };
    if cr != CR_SUCCESS {
        return Err(cr_err("CM_Get_Device_Interface_List_SizeW", cr.0));
    }
    let mut buf = vec![0u16; len as usize];
    let cr = unsafe {
        CM_Get_Device_Interface_ListW(guid, id_ptr, &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
    };
    if cr != CR_SUCCESS {
        return Err(cr_err("CM_Get_Device_Interface_ListW", cr.0));
    }
    Ok(buf
        .split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect())
}

/// Device instance ID string for a device-interface path.
fn interface_instance_id(interface: &str) -> io::Result<Vec<u16>> {
    let iface = wide(interface);
    let mut prop_type = windows::Win32::Devices::Properties::DEVPROPTYPE(0);
    let mut buf = vec![0u8; 1024];
    let mut size = buf.len() as u32;
    let cr = unsafe {
        windows::Win32::Devices::DeviceAndDriverInstallation::CM_Get_Device_Interface_PropertyW(
            PCWSTR(iface.as_ptr()),
            &DEVPKEY_Device_InstanceId as *const _ as *const DEVPROPKEY,
            &mut prop_type,
            Some(buf.as_mut_ptr()),
            &mut size,
            0,
        )
    };
    if cr != CR_SUCCESS {
        return Err(cr_err("CM_Get_Device_Interface_PropertyW", cr.0));
    }
    let words: Vec<u16> = buf[..size as usize]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Ok(words)
}

fn locate_devnode(id: &[u16]) -> io::Result<u32> {
    let mut devinst = 0u32;
    let cr =
        unsafe { CM_Locate_DevNodeW(&mut devinst, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) };
    if cr != CR_SUCCESS {
        return Err(cr_err("CM_Locate_DevNodeW", cr.0));
    }
    Ok(devinst)
}

fn device_id(devinst: u32) -> io::Result<String> {
    let mut len = 0u32;
    let cr = unsafe { CM_Get_Device_ID_Size(&mut len, devinst, 0) };
    if cr != CR_SUCCESS {
        return Err(cr_err("CM_Get_Device_ID_Size", cr.0));
    }
    let mut buf = vec![0u16; len as usize + 1];
    let cr = unsafe { CM_Get_Device_IDW(devinst, &mut buf, 0) };
    if cr != CR_SUCCESS {
        return Err(cr_err("CM_Get_Device_IDW", cr.0));
    }
    Ok(String::from_utf16_lossy(
        &buf[..len as usize],
    ))
}

fn parent(devinst: u32) -> io::Result<u32> {
    let mut p = 0u32;
    let cr = unsafe { CM_Get_Parent(&mut p, devinst, 0) };
    if cr != CR_SUCCESS {
        return Err(cr_err("CM_Get_Parent", cr.0));
    }
    Ok(p)
}

/// SPDRP_ADDRESS via cfgmgr: for a device on a USB hub this is the port number.
fn devnode_address(devinst: u32) -> io::Result<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let cr = unsafe {
        CM_Get_DevNode_Registry_PropertyW(
            devinst,
            CM_DRP_ADDRESS,
            None,
            Some(&mut value as *mut _ as *mut _),
            &mut size,
            0,
        )
    };
    if cr != CR_SUCCESS {
        return Err(cr_err("CM_Get_DevNode_Registry_PropertyW(ADDRESS)", cr.0));
    }
    Ok(value)
}

/// Find the physical-disk device instance backing a volume.
fn disk_devinst_for_drive(drive: &Drive) -> io::Result<u32> {
    let volume = open_query(&format!(r"\\.\{}:", drive.letter))?;
    let vol_num = device_number(&volume)?;
    drop(volume);

    for iface in interface_list(&GUID_DEVINTERFACE_DISK, None)? {
        let Ok(disk) = open_query(&iface) else {
            continue;
        };
        let Ok(num) = device_number(&disk) else {
            continue;
        };
        if num.DeviceType == vol_num.DeviceType && num.DeviceNumber == vol_num.DeviceNumber {
            let id = interface_instance_id(&iface)?;
            return locate_devnode(&id);
        }
    }
    Err(Error::new(
        ErrorKind::NotFound,
        format!("no physical disk found for {}", drive.display()),
    ))
}

/// Walk up from the disk to the node whose device ID is USB\VID_....
fn find_usb_ancestor(mut devinst: u32) -> io::Result<(u32, String)> {
    for _ in 0..8 {
        devinst = match parent(devinst) {
            Ok(p) => p,
            Err(_) => break,
        };
        let id = device_id(devinst)?;
        if id.to_ascii_uppercase().starts_with(r"USB\VID_") {
            return Ok((devinst, id));
        }
    }
    Err(Error::new(
        ErrorKind::NotFound,
        "drive is not attached over USB (no USB ancestor in the device tree)",
    ))
}

fn open_hub(hub_devinst: u32) -> io::Result<(File, String)> {
    let hub_id = device_id(hub_devinst)?;
    let hub_id_w = wide(&hub_id);
    let interfaces = interface_list(&GUID_DEVINTERFACE_USB_HUB, Some(&hub_id_w))?;
    let path = interfaces.into_iter().next().ok_or_else(|| {
        Error::new(
            ErrorKind::NotFound,
            format!("parent node {hub_id} exposes no USB hub interface"),
        )
    })?;
    let hub = OpenOptions::new()
        .access_mode(GENERIC_WRITE)
        .share_mode(FILE_SHARE_WRITE.0)
        .open(&path)?;
    Ok((hub, hub_id))
}

fn connection_info(hub: &File, port: u32) -> io::Result<USB_NODE_CONNECTION_INFORMATION_EX> {
    let mut info = USB_NODE_CONNECTION_INFORMATION_EX::default();
    info.ConnectionIndex = port;
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            HANDLE(hub.as_raw_handle()),
            IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX,
            Some(&info as *const _ as *const _),
            size_of::<USB_NODE_CONNECTION_INFORMATION_EX>() as u32,
            Some(&mut info as *mut _ as *mut _),
            size_of::<USB_NODE_CONNECTION_INFORMATION_EX>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(|e| Error::other(format!("IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX: {e}")))?;
    Ok(info)
}

/// Flags word from the V2 connection query; None when the hub predates USB 3.
fn connection_flags_v2(hub: &File, port: u32) -> Option<u32> {
    let mut info = USB_NODE_CONNECTION_INFORMATION_EX_V2::default();
    info.ConnectionIndex = port;
    info.Length = size_of::<USB_NODE_CONNECTION_INFORMATION_EX_V2>() as u32;
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            HANDLE(hub.as_raw_handle()),
            IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2,
            Some(&info as *const _ as *const _),
            info.Length,
            Some(&mut info as *mut _ as *mut _),
            info.Length,
            Some(&mut returned),
            None,
        )
    };
    match ok {
        Ok(()) => Some(unsafe { info.Flags.ul }),
        Err(_) => None,
    }
}

/// Fetch a USB string descriptor through the hub. Best-effort: returns None on
/// any failure — string descriptors are optional and some devices stall them.
fn string_descriptor(hub: &File, port: u32, index: u8) -> Option<String> {
    if index == 0 {
        return None;
    }
    const BUF: usize = 512;
    let mut raw = vec![0u8; size_of::<USB_DESCRIPTOR_REQUEST>() + BUF];
    let req = raw.as_mut_ptr() as *mut USB_DESCRIPTOR_REQUEST;
    unsafe {
        (*req).ConnectionIndex = port;
        (*req).SetupPacket.wValue = ((USB_STRING_DESCRIPTOR_TYPE as u16) << 8) | index as u16;
        (*req).SetupPacket.wIndex = 0x0409; // en-US
        (*req).SetupPacket.wLength = BUF as u16;
    }
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            HANDLE(hub.as_raw_handle()),
            IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION,
            Some(raw.as_ptr() as *const _),
            raw.len() as u32,
            Some(raw.as_mut_ptr() as *mut _),
            raw.len() as u32,
            Some(&mut returned),
            None,
        )
    };
    if ok.is_err() {
        return None;
    }
    // Past the request header sits USB_STRING_DESCRIPTOR: bLength,
    // bDescriptorType, then UTF-16LE payload.
    let data = &raw[size_of::<USB_DESCRIPTOR_REQUEST>()..];
    let b_length = *data.first()? as usize;
    if b_length < 2 || data.get(1) != Some(&USB_STRING_DESCRIPTOR_TYPE) {
        return None;
    }
    let payload = data.get(2..b_length)?;
    let words: Vec<u16> = payload
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let s = String::from_utf16_lossy(&words).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Full USB link + identity report for a drive letter.
pub fn for_drive(drive: &Drive) -> io::Result<UsbInfo> {
    let disk = disk_devinst_for_drive(drive)?;
    let (usb_devinst, instance_id) = find_usb_ancestor(disk)?;
    let port = devnode_address(usb_devinst)?;
    let (hub, hub_instance_id) = open_hub(parent(usb_devinst)?)?;

    let info = connection_info(&hub, port)?;
    let desc = info.DeviceDescriptor;
    let v2_flags = connection_flags_v2(&hub, port);

    // Speed values 0..2 are USB_DEVICE_SPEED low/full/high; 3 means SuperSpeed
    // "or higher" — the V2 flags distinguish plain 5 Gbps from 10+ Gbps
    // (bit 2: operating at SuperSpeedPlus or higher).
    let negotiated = match info.Speed {
        0 => LinkSpeed::Low,
        1 => LinkSpeed::Full,
        2 => LinkSpeed::High,
        _ => match v2_flags {
            Some(f) if f & 0b100 != 0 => LinkSpeed::SuperPlus,
            _ => LinkSpeed::Super,
        },
    };

    Ok(UsbInfo {
        instance_id,
        vid: desc.idVendor,
        pid: desc.idProduct,
        manufacturer: string_descriptor(&hub, port, desc.iManufacturer),
        product: string_descriptor(&hub, port, desc.iProduct),
        serial: string_descriptor(&hub, port, desc.iSerialNumber),
        bcd_usb: desc.bcdUSB,
        negotiated,
        port,
        hub_instance_id,
    })
}
