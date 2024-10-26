# `spinup`

`spinup` is a lightweight Wake-On-LAN (WOL) scheduler and dependency manager designed for small businesses and homelabs. It ensures that infrastructure services like firewalls, storage, and hypervisors are brought online in the correct order, particularly after events like power outages.

A typical setup might involve configuring most of the infrastructure for WOL but not Wake-On-Power and configuring `spinup` to run on a low-power device, such as a Raspberry Pi. When you need to bring the entire environment online, simply power on the host running wakeup, and the rest of the infrastructure will automatically follow in the correct order.

## Features

- [x] *VLAN Support*: Send WOL packets to devices across different VLANs.
- [x] *YAML Configuration*: Easily define server boot sequences, dependencies, and status checks.
- [ ] *Service Status Checks*: Verify that a service is up using built-in status checks (HTTP health checks, NFS, SMB, custom shell commands).
    - [x] HTTP
    - [ ] Open port
    - [x] Shell
    - [ ] NFS (might just use open port check)
    - [ ] SMB (might just use open port check)
- [ ] *Plugin-Friendly*: Users can write their own custom status check plugins.

## Configuration

Sample configuration

> Still a work in progress!!

```yaml
- name: "Firewall"
  mac: "00:1A:2B:3C:4D:5E"
  vlan: 10
  depends: []
  check:
    - type: "http"
      url: "http://192.168.1.1/health"
      expected_status: 200

- name: "Storage Server 1"
  mac: "00:1A:2B:3C:4D:5F"
  vlan: 100
  depends:
    - "Firewall"
  check:
    - type: "nfs"
    - 
- name: "Storage Server 2"
  mac: "00:1A:2B:3C:4D:5G"
  vlan: 100
  depends:
    - "Firewall"
  check:
    - type: "smb"

- name: "VM Host"
  mac: "00:1A:2B:3C:4D:60"
  vlan: 200
  depends:
    - "Storage Server 1"
    - "Storage Server 2"
  check:
    - type: "http"
      response: 200
```

## License

This project is licensed under either of the following licenses, at your option:

- [MIT License](./LICENSE-MIT)
- [Apache License 2.0](./LICENSE-APACHE)

You may choose to use this project under the terms of either license.
