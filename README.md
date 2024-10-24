# `wakeup`

`wakeup` is a lightweight Wake-On-LAN (WOL) scheduler and dependency manager intended for use in small businesses and homelabs.
We can define the service dependencies between the servers to make sure that infrastructure services (firewalls, storage, hypervisors, etc.) are brought online in the correct order after events such as power outages.

## Features

- [ ] *VLAN Support*: Send WOL packets to devices across different VLANs.
- [ ] *YAML Configuration*: Easily define server boot sequences, dependencies, and status checks.
- [ ] *Service Status Checks*: Verify that a service is up using built-in status checks (HTTP health checks, NFS, SMB, custom shell commands).
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
