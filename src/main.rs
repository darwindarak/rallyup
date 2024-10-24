use pnet::datalink::{self, Channel::Ethernet};
use pnet::packet::ethernet::{EtherTypes, MutableEthernetPacket};
use pnet::packet::{MutablePacket, Packet};
use pnet::util::MacAddr;

const SIZE_DST_MAC: usize = 6;
const SIZE_SRC_MAC: usize = 6;
const SIZE_ETHERTYPE: usize = 2;
const SIZE_VLAN_ETHERTYPE: usize = 2;
const SIZE_VLAN_TAG: usize = 2;
const SIZE_WOL_PAYLOAD: usize = 102;

const WOL_ETHERTYPE: [u8; 2] = [0x08, 0x42];

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
    let vlan_tag = ((vlan & 0x0FFF) as u16).to_be_bytes();
    return vlan_tag.to_vec();
}

fn send_wol_packet(
    maybe_mac: &str,
    interface_name: &str,
    vlan_id: Option<u16>,
) -> Result<(), std::io::Error> {
    let mac = maybe_mac.parse::<MacAddr>().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("cannot parse {} as a MAC address: {}", maybe_mac, e),
        )
    })?;
    let wol_packet = create_wol_payload(mac);

    let interface = datalink::interfaces()
        .into_iter()
        .find(|iface| iface.name == interface_name)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("interface `{}` not found", interface_name),
            )
        })?;

    let mut tx = match datalink::channel(&interface, Default::default()) {
        Ok(Ethernet(tx, _)) => tx,
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "unhandled channel type for this interface",
            ))
        }
        Err(e) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("failed to open channel: {}", e),
            ))
        }
    };

    let payload_size = if vlan_id.is_some() {
        SIZE_VLAN_TAG + SIZE_VLAN_ETHERTYPE
    } else {
        0
    } + SIZE_WOL_PAYLOAD;

    let packet_size = SIZE_DST_MAC + SIZE_SRC_MAC + SIZE_ETHERTYPE + payload_size;
    let mut buffer = vec![0u8; packet_size];

    let mut packet = MutableEthernetPacket::new(&mut buffer[..]).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            "failed to create ethernet packet",
        )
    })?;

    packet.set_destination(MacAddr::broadcast());

    if let Some(mac) = interface.mac {
        packet.set_source(mac);
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "failed to get source MAC address of the interface",
        ));
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

    tx.send_to(packet.packet(), None).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "failed to send WOL packet")
    })??;

    println!(
        "WOL packet sent successfully over interface: {}",
        interface_name
    );

    Ok(())
}

fn main() {
    if let Err(error) = send_wol_packet("AA:BB:CC:DD:EE:FF", "en11", Some(129)) {
        println!("{}", error);
    }

    println!("Hello, world!");
}
