# `rallyup`

`rallyup` is a lightweight Wake-On-LAN (WOL) scheduler and dependency manager designed for small businesses and homelabs. It ensures that infrastructure services like firewalls, storage, and hypervisors are brought online in the correct order, particularly after events like power outages.

A typical setup involves configuring most of the infrastructure for WOL but not for Wake-On-Power, and setting `rallyup` to run on startup on a low-power device like a Raspberry Pi. When you need to bring the entire environment online, simply power on the device running `rallyup`, and the rest of the infrastructure will automatically follow in the correct order.

![Tests](https://github.com/darwindarak/rallyup/actions/workflows/tests.yml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/rallyup.svg)](https://crates.io/crates/rallyup)

## Features

- [x] *VLAN Support*: Send WOL packets to devices across different VLANs.
- [x] *YAML Configuration*: Easily define server boot sequences, dependencies, and status checks.
- [ ] *Service Status Checks*: Verify that a service is up using built-in status checks (HTTP health checks, NFS, SMB, custom shell commands).
    - [x] HTTP
    - [x] Open port
    - [x] Shell
    - [ ] NFS (might just use open port check)
    - [ ] SMB (might just use open port check)
- [ ] *Plugin-Friendly*: Users can write their own custom status check plugins.

## Usage

```sh
rallyup servers.yaml
```

## Configuration

The dependencies between servers, along with the methods for validating that they are online, are defined in a YAML configuration file.

## Servers Configuration

**Fields**:
- **name**: The name of the server, used for identification when defining dependencies between servers
- **mac**: The MAC address of the server we want to wake up
- **interface**: The network interface to use when sending the WOL packet
- **vlan**: The VLAN ID (optional) that the server is on
- **depends**: A list of other server names that this server depends on
- **check**: A list of health checks that must pass before this server is considered fully online

**Example**:
```yaml
- name: "firewall"
  mac: "00:11:22:33:44:55"
  interface: "eth0"
  vlan: 100
  depends:
    - "storage"
  check: [... see below]
```
- 
## Health Check Configurations

Each server can have multiple health checks to ensure the server is fully online before the next device starts up.

### Common Fields

- **retry**: The interval, defined in human readable string (e.g. 1s, 1 minute, etc.) to wait between retrying this health check
- **timeout**: The timeout interval after which the check, and subsequently the entire boot sequence, will fail

### Built-in Health Checks

#### HTTP Health Checks

The HTTP health check verifies whether a specified endpoint responds as expected.

**Fields**
- **type**: should be `http` for an HTTP health check.
- **url**:  The URL to perform the HTTP health check against
- **status**: Expected HTTP status code
- **regex**: Regex to match in the response body 

> Note: You must provide either `status` or `regex`, or both.

**Example**
```yaml
- type: http
  url: "http://192.168.1.1/health"
  status: 200
  retry: 5s
  timeout: 30s
```

#### Port Health Check

The port health check verifies whether a specified TCP port on a server is open and accessible. 
This is really a stand-in for verifying NFS and SMB ports until I can figure out how to check if those services are online.

**Fields**
- **type**: should be `port` for a port health check
- **ip**: the IP address to check
- **port**: the port number to check

**Example**
```yaml
- type: port
  ip: "192.168.1.1"
  port: 22
  retry: "10s"
  timeout: "1m"
```

#### Shell Health Checks

The shell health check executes a shell command checks the result.
This is to provide the option of user-defined health checks.

**Fields**
- **type**: should be `shell` for a shell health check.
- **command**:  he shell command to execute
- **status**: Expected exit code
- **regex**: Regex to match in the standard output

> Note: You must provide either `status` or `regex`, or both.

**Example**
```yaml
- type: shell
  command: ping -c 1 192.168.1.1
  status: 0
  retry: 5s
  timeout: 20s
```

### Full Example

> TODO:
> - [ ] Need to test in the lab and post the actual sample

```yaml
- name: "Firewall"
  mac: "00:1A:2B:3C:4D:5E"
  interface: eth0
  vlan: 10
  depends: []
  check:
    - type: http
      url: "http://192.168.1.1/health"
      status: 200
      regex: 'ok'

- name: "Storage Server 1"
  mac: "00:1A:2B:3C:4D:5F"
  interface: eth0
  vlan: 100
  depends:
    - "Firewall"
  check:
    - type: port
      ip: 192.168.100.101
      port: 2049
      timeout: 5 minutes

- name: "Storage Server 2"
  mac: "00:1A:2B:3C:4D:5G"
  vlan: 100
  depends:
    - "Firewall"
  check:
    - type: port
      ip: 192.168.100.102
      port: 445
      retry: 5s

- name: "VM Host"
  mac: "00:1A:2B:3C:4D:60"
  vlan: 200
  depends:
    - "Storage Server 1"
    - "Storage Server 2"
  check:
    - type: command
      command: "ping -c 192.168.200.10"
      status 0
```

## License

This project is licensed under either of the following licenses, at your option:

- [MIT License](./LICENSE-MIT)
- [Apache License 2.0](./LICENSE-APACHE)

You may choose to use this project under the terms of either license.
