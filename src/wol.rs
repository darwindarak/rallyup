use pnet::datalink::NetworkInterface;
use pnet::datalink::{self, Channel::Ethernet};
use pnet::packet::ethernet::{EtherTypes, EthernetPacket, MutableEthernetPacket};
use pnet::packet::{MutablePacket, Packet};
use pnet::util::MacAddr;

use thiserror::Error;

const SIZE_DST_MAC: usize = 6;
const SIZE_SRC_MAC: usize = 6;
const SIZE_ETHERTYPE: usize = 2;
const SIZE_VLAN_ETHERTYPE: usize = 2;
const SIZE_VLAN_TAG: usize = 2;
const SIZE_WOL_PAYLOAD: usize = 102;

const WOL_ETHERTYPE: [u8; 2] = [0x08, 0x42];

#[derive(Debug, Error)]
pub enum WOLError {
    #[error("Invalid MAC address: {0}")]
    InvalidMAC(String),

    #[error("Failed to find network interface: {0}")]
    InterfaceNotFound(String),

    #[error("Network error: {0}")]
    NetworkError(#[from] std::io::Error),

    #[error("Failed to send WOL packet for server: {0}")]
    WOLPacketError(String),
}

type Result<T> = std::result::Result<T, WOLError>;

// Ethernet Frame Layout:
// -----------------------------------------------------------------------------
// | Destination MAC  | Source MAC      | VLAN EtherType | VLAN Tag | WOL EtherType | WOL Magic Packet                              |
// | (Broadcast MAC)  | (Interface MAC) | (0x8100)       | (VLAN ID)| (0x0842)      | (FF:FF:FF + 16 * Target MAC)                  |
// -----------------------------------------------------------------------------
// | 6 bytes          | 6 bytes         | 2 bytes        | 2 bytes  | 2 bytes       | 102 bytes                                     |
// -----------------------------------------------------------------------------
// Detailed Breakdown of Each Component:
// - Destination MAC (6 bytes): The destination MAC address, usually the broadcast MAC (FF:FF:FF:FF:FF:FF) for WOL packets.
// - Source MAC (6 bytes): The source MAC address, which is the MAC address of the sending interface.
// - VLAN EtherType (2 bytes): The EtherType field for VLAN tagging, which is always 0x8100 to indicate the presence of a VLAN tag.
// - VLAN Tag (2 bytes): The VLAN tag, which contains 12 bits for the VLAN ID and 4 bits for priority and CFI (Canonical Format Indicator).
// - WOL EtherType (2 bytes): The EtherType field indicating a Wake-on-LAN packet, which is 0x0842.
// - WOL Magic Packet (102 bytes): The WOL magic packet, consisting of 6 bytes of FF followed by the target MAC address repeated 16 times.

fn create_wol_payload(mac: MacAddr) -> Vec<u8> {
    // 6 bytes of FF followed by target MAC address repeated 16 times
    let mut packet = vec![0xFF; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac.octets());
    }
    packet
}

fn vlan_to_bytes(vlan: u16) -> Vec<u8> {
    // Do not need priority bits, using only the remaining 14 bits of the tag
    let vlan_tag = (vlan & 0x0FFF).to_be_bytes();
    vlan_tag.to_vec()
}

pub fn build_wol_packet(
    maybe_mac: &str,
    interface_name: &str,
    vlan_id: Option<u16>,
) -> Result<(Vec<u8>, NetworkInterface)> {
    let mac = maybe_mac
        .parse::<MacAddr>()
        .map_err(|_| WOLError::InvalidMAC(maybe_mac.to_string()))?;

    let wol_packet = create_wol_payload(mac);

    let interface = datalink::interfaces()
        .into_iter()
        .find(|iface| iface.name == interface_name)
        .ok_or_else(|| WOLError::InterfaceNotFound(interface_name.to_string()))?;

    let payload_size = if vlan_id.is_some() {
        SIZE_VLAN_TAG + SIZE_VLAN_ETHERTYPE
    } else {
        0
    } + SIZE_WOL_PAYLOAD;

    let packet_size = SIZE_DST_MAC + SIZE_SRC_MAC + SIZE_ETHERTYPE + payload_size;
    let mut buffer = vec![0u8; packet_size];

    let mut packet = MutableEthernetPacket::new(&mut buffer[..])
        .ok_or_else(|| WOLError::WOLPacketError("failed to create ethernet packet".into()))?;

    packet.set_destination(MacAddr::broadcast());

    if let Some(mac) = interface.mac {
        packet.set_source(mac);
    } else {
        return Err(WOLError::NetworkError(std::io::Error::new(
            std::io::ErrorKind::Other,
            "failed to get source MAC address of the interface",
        )));
    }

    let payload_offset = if let Some(vlan) = vlan_id {
        packet.set_ethertype(EtherTypes::Vlan);

        let vlan_tag = vlan_to_bytes(vlan);
        packet.payload_mut()[..SIZE_VLAN_TAG].copy_from_slice(&vlan_tag);

        // Set WOL Ethertype manually
        packet.payload_mut()[SIZE_VLAN_TAG..(SIZE_VLAN_TAG + SIZE_ETHERTYPE)]
            .copy_from_slice(&WOL_ETHERTYPE);

        SIZE_VLAN_TAG + SIZE_ETHERTYPE
    } else {
        packet.set_ethertype(EtherTypes::WakeOnLan);
        0
    };

    packet.payload_mut()[payload_offset..].copy_from_slice(&wol_packet);

    Ok((buffer, interface))
}

pub fn send_wol_packet(maybe_mac: &str, interface_name: &str, vlan_id: Option<u16>) -> Result<()> {
    let (packet_buffer, interface) = build_wol_packet(maybe_mac, interface_name, vlan_id)?;

    let packet = EthernetPacket::new(&packet_buffer)
        .expect("`packet_buffer` was created by a `MutableEthernetPacket`, should not error here");

    let mut tx = match datalink::channel(&interface, Default::default()) {
        Ok(Ethernet(tx, _)) => tx,
        Ok(_) => {
            return Err(WOLError::NetworkError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "unhandled channel type for this interface",
            )))
        }
        Err(e) => return Err(WOLError::NetworkError(e)),
    };

    tx.send_to(packet.packet(), None).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "failed to send WOL packet")
    })??;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_mac_address() {
        let invalid_mac = "random MAC";
        let interface_name = "eth0";

        let result = send_wol_packet(invalid_mac, interface_name, None);
        assert!(
            matches!(result, Err(WOLError::InvalidMAC(_))),
            "Expected InvalidMAC error."
        );
    }

    #[test]
    fn test_missing_network_interface() {
        let mac = "00:11:22:33:44:55";
        let non_existent_interface = "nonexistent_iface";

        let result = send_wol_packet(mac, non_existent_interface, None);
        assert!(
            matches!(result, Err(WOLError::InterfaceNotFound(_))),
            "Expected InterfaceNotFound error."
        );
    }

    #[test]
    fn test_ethernet_packet() {
        let maybe_mac = "01:23:45:67:89:AB";
        let mac = maybe_mac.parse::<MacAddr>().unwrap();
        let dest_mac = MacAddr::broadcast();
        //let vlan_id = Some(100);

        let interface = datalink::interfaces()
            .into_iter()
            .find(|iface| iface.mac.is_some())
            .expect("cannot find an interface with a MAC address for testing");

        let payload_size = SIZE_WOL_PAYLOAD;
        let packet_size = SIZE_DST_MAC + SIZE_SRC_MAC + SIZE_ETHERTYPE + payload_size;

        let (buffer, _interface) = build_wol_packet(maybe_mac, &interface.name, None)
            .expect("failed to build test packet");

        assert_eq!(packet_size, buffer.len());

        // L2 broadcast address
        assert_eq!(dest_mac.octets(), buffer[..6]);
        // Source MAC
        assert_eq!(interface.mac.unwrap().octets(), buffer[6..12]);
        // WOL Ethertype
        assert_eq!(vec![0x08, 0x42], buffer[12..14]);
        // WOL packet prefix
        assert_eq!(vec![0xFF; 6], buffer[14..20]);
        // WOL target MAC x 16
        assert_eq!(mac.octets().repeat(16), buffer[20..]);
    }

    #[test]
    fn test_ethernet_packet_with_vlan() {
        let maybe_mac = "01:23:45:67:89:AB";
        let mac = maybe_mac.parse::<MacAddr>().unwrap();
        let dest_mac = MacAddr::broadcast();
        let vlan_id = Some(0x0101);

        let interface = datalink::interfaces()
            .into_iter()
            .find(|iface| iface.mac.is_some())
            .expect("cannot find an interface with a MAC address for testing");

        let payload_size = SIZE_VLAN_TAG + SIZE_VLAN_ETHERTYPE + SIZE_WOL_PAYLOAD;
        let packet_size = SIZE_DST_MAC + SIZE_SRC_MAC + SIZE_ETHERTYPE + payload_size;

        let (buffer, _interface) = build_wol_packet(maybe_mac, &interface.name, vlan_id)
            .expect("failed to build test packet");

        assert_eq!(packet_size, buffer.len());

        // L2 broadcast address
        assert_eq!(dest_mac.octets(), buffer[..6]);
        // Source MAC
        assert_eq!(interface.mac.unwrap().octets(), buffer[6..12]);
        // VLAN Ethertype
        assert_eq!(vec![0x81, 0x00], buffer[12..14]);
        // VLAN Tag
        assert_eq!(vec![0x01, 0x01], buffer[14..16]);
        // WOL Ethertype
        assert_eq!(vec![0x08, 0x42], buffer[16..18]);
        // WOL packet prefix
        assert_eq!(vec![0xFF; 6], buffer[18..24]);
        // WOL target MAC x 16
        assert_eq!(mac.octets().repeat(16), buffer[24..]);
    }
}
