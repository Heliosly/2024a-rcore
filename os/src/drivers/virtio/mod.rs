#![no_std]
#![feature(used_with_arg)]

extern crate alloc;
#[macro_use]

pub mod virtio_blk;
pub mod virtio_impl;
pub mod virtio_input;
pub mod virtio_net;
// pub mod loongson;
use core::ptr::NonNull;

use alloc::{sync::Arc, vec::Vec};
use crate::devices::{
    device::{Driver, UnsupportedDriver},
    fdt::Node,
    node_to_interrupts, VIRT_ADDR_START,
};
#[cfg(target_arch = "loongarch64")]
use crate::driver_define;
use virtio_drivers::transport::{
    mmio::{self, MmioTransport, VirtIOHeader},
    DeviceType, Transport,
};

#[cfg(any(target_arch = "x86_64", target_arch = "loongarch64"))]
use crate::devices::ALL_DEVICES;
#[cfg(any(target_arch = "x86_64", target_arch = "loongarch64"))]
use virtio_drivers::transport::pci::bus::MmioCam;

#[cfg(any(target_arch = "x86_64", target_arch = "loongarch64"))]
use virtio_drivers::transport::pci::{
    bus::{BarInfo, Cam, Command, DeviceFunction, PciRoot},
    virtio_device_type, PciTransport,
};


pub fn init_mmio(node: &Node) -> Arc<dyn Driver> {
    if let Some(reg) = node.reg().and_then(|mut reg| reg.next()) {
        let paddr = reg.address as usize;
        let vaddr = VIRT_ADDR_START + paddr;
        let header = NonNull::new(vaddr as *mut VirtIOHeader).unwrap();
        if let Ok(transport) = unsafe { MmioTransport::new(header) } {
            info!(
                "Detected virtio MMIO device with
                    vendor id {:#X}
                    device type {:?}
                    version {:?}
                    addr @ {:#X}
                    interrupt: {:?}",
                transport.vendor_id(),
                transport.device_type(),
                transport.version(),
                vaddr,
                node.interrupts().unwrap().flatten().collect::<Vec<u32>>()
            );
            return virtio_device(transport, node);
        }
    }
    Arc::new(UnsupportedDriver)
}

fn virtio_device(transport: MmioTransport, node: &Node) -> Arc<dyn Driver> {
    let irqs = node_to_interrupts(node);
    match transport.device_type() {
        DeviceType::Block => virtio_blk::init(transport, irqs),
        DeviceType::Input => virtio_input::init(transport, irqs),
        DeviceType::Network => virtio_net::init(transport, irqs),
        device_type => {
            warn!("Unrecognized virtio device: {:?}", device_type);
            Arc::new(UnsupportedDriver)
        }
    }
}
#[cfg(any(target_arch = "x86_64", target_arch = "loongarch64"))]
fn enumerate_pci(mmconfig_base: *mut u8) {
    info!("mmconfig_base = {:#x}", mmconfig_base as usize);

    let mut pci_root = unsafe { PciRoot::<MmioCam>::new(MmioCam::new(mmconfig_base, Cam::Ecam)) };
    for (device_function, info) in pci_root.enumerate_bus(0) {
        let (status, command) = pci_root.get_status_command(device_function);
        info!(
            "Found {} at {}, status {:?} command {:?}",
            info, device_function, status, command
        );
        if let Some(virtio_type) = virtio_device_type(&info) {
            use crate::drivers::virtio::virtio_impl::HalImpl;

            info!("  VirtIO {:?}", virtio_type);

            // Enable the device to use its BARs.
            pci_root.set_command(
                device_function,
                Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
            );
            dump_bar_contents(&mut pci_root, device_function, 4);

            let mut transport =
                PciTransport::new::<HalImpl, MmioCam>(&mut pci_root, device_function).unwrap();
                let dev_features = transport.read_device_features();
                let filtered = dev_features & !(0x10000000 | 0x20000000); // 硬编码屏蔽
                 transport.write_driver_features(filtered);
            info!(
                "Detected virtio PCI device with device type {:?}, features {:#018x}",
                transport.device_type(),
                transport.read_device_features(),
            );
            virtio_device_probe(transport);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "loongarch64"))]
fn virtio_device_probe(transport: impl Transport + 'static) {
    let device = match transport.device_type() {
        DeviceType::Block => Some(virtio_blk::init(transport, Vec::new())),
        // DeviceType::Input => virtio_input::init(transport, Vec::new()),
        DeviceType::Network => Some(virtio_net::init(transport, Vec::new())),
        t => {
            warn!("Unrecognized virtio device: {:?}", t);
            None
        }
    };

    if let Some(device) = device {
        info!("is locked: {}", ALL_DEVICES.is_locked());
        ALL_DEVICES.lock().add_device(device);
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "loongarch64"))]
fn dump_bar_contents(root: &mut PciRoot<MmioCam>, device_function: DeviceFunction, bar_index: u8) {
    let bar_info = root.bar_info(device_function, bar_index).unwrap();
    trace!("Dumping bar {}: {:#x?}", bar_index, bar_info);

    #[cfg(target_arch = "loongarch64")]
    if let BarInfo::Memory {
        address,
        size,
        address_type,
        ..
    } = bar_info
    {
        if address == 0 && size > 0 {
            use spin::Mutex;
            use virtio_drivers::transport::pci::bus::MemoryBarType;

            // 指定 PCI BAR 映射可用的内存范围
            static PCI_RANGES: Mutex<(usize, usize)> = Mutex::new((0x4000_0000, 0x2_0000));
            assert!(PCI_RANGES.lock().1 > size as usize);
            let start = PCI_RANGES.lock().0;
            PCI_RANGES.lock().1 -= size as usize;
            PCI_RANGES.lock().0 += size as usize;

            match address_type {
                MemoryBarType::Width32 => root.set_bar_32(device_function, bar_index, start as _),
                MemoryBarType::Below1MiB => todo!(),
                MemoryBarType::Width64 => root.set_bar_64(device_function, bar_index, start as _),
            }
        }
    }

    let bar_info = root.bar_info(device_function, bar_index).unwrap();

    if let BarInfo::Memory { address, size, .. } = bar_info {
        let start = (address as usize | VIRT_ADDR_START) as *const u8;

        unsafe {
            let mut buf = [0u8; 32];
            for i in 0..size / 32 {
                let ptr = start.add(i as usize * 32);
                core::ptr::copy(ptr, buf.as_mut_ptr(), 32);
                if buf.iter().any(|b| *b != 0xff) {
                    trace!("  {:?}: {:x?}", ptr, buf);
                }
            }
        }
    }
    trace!("End of dump");
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "loongarch64")))]
driver_define!("virtio,mmio", init_mmio);

#[cfg(target_arch = "x86_64")]
driver_define!({
    enumerate_pci((0xB000_0000usize | VIRT_ADDR_START) as _);
    None
});

#[cfg(target_arch = "loongarch64")]
driver_define!({
    enumerate_pci((0x2000_0000usize | VIRT_ADDR_START) as _);
    None
});
