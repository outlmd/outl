import Foundation

/// Bonjour-backed LAN peer discovery for iOS (issue #149).
///
/// ## Why iOS does not use the same path as every other platform
///
/// Every other client discovers same-LAN peers through
/// `iroh-mdns-address-lookup`, which speaks mDNS over a plain BSD socket:
/// `bind` on `0.0.0.0:5353`, then `IP_ADD_MEMBERSHIP` on `224.0.0.251`.
/// Since iOS 14 that join needs `com.apple.developer.networking.multicast`, a
/// restricted entitlement Apple grants by request — and which Apple commonly
/// declines, with the suggestion to use Bonjour instead.
///
/// So iOS goes through `mDNSResponder`, the system's own mDNS daemon, via
/// `NetService` / `NetServiceBrowser`. Apple's Local Network Privacy FAQ is
/// explicit that only two Bonjour operations need the entitlement: working
/// with **arbitrary** service types, and browsing for advertised service types
/// (the `_services._dns-sd._udp.local.` meta-query). This file does neither —
/// it uses one fixed service type, declared in `NSBonjourServices`. No
/// entitlement, no request form, no approval to wait on.
///
/// ## Why this interoperates with the other clients
///
/// `swarm-discovery` publishes plain DNS-SD (RFC 6763), so there is no private
/// protocol to reimplement:
///
/// - service type `_irohv1._udp.local.`
/// - instance name = the endpoint id in lowercase base32
///   (`outl_sync_iroh::lan::instance_label`), **not** iroh's `Display`, which
///   is 64 hex chars and one byte over RFC 6763's 63-byte cap on a DNS label
/// - a TXT record whose `relay` key holds the home relay URL
/// - SRV + A/AAAA for the direct addresses
///
/// A Mac publishing through `swarm-discovery` and an iPhone browsing through
/// `mDNSResponder` therefore see each other's records unchanged. Any drift in
/// the constants below breaks that silently and in both directions, so they are
/// pinned on the Rust side by `the_lan_wire_constants_match_the_platform_bridge`.
///
/// ## Why `NetService` and not `NWListener`
///
/// `NWListener` opens its own port and advertises *that*. The port we must
/// advertise is the one iroh's QUIC socket already holds, and no
/// `Network.framework` type will advertise a socket it did not create.
/// `NetService(domain:type:name:port:)` publishes a record for an arbitrary
/// port, which is exactly the job. It is soft-deprecated, not removed, and
/// deprecation does not affect App Review.
///
/// Browsing has the same constraint from the other side: `NWBrowser` hands back
/// an opaque `NWEndpoint.service`, by design — the framework wants you to
/// connect through it rather than learn addresses. iroh needs the actual
/// `SocketAddr`s to hand to QUIC, and `NetService.addresses` is what exposes
/// them.
///
/// ## Threading
///
/// Every `NetService` / `NetServiceBrowser` call and every delegate callback
/// runs on the main run loop, because that is what these APIs schedule
/// themselves on. The Rust callback is invoked from there; it only sends on a
/// channel, so it does not block the main thread.
final class OutlBonjour: NSObject {
  /// Must equal `outl_sync_iroh::lan::SERVICE_TYPE`.
  private static let serviceType = "_irohv1._udp."
  private static let domain = "local."
  /// Must equal `outl_sync_iroh::lan::RELAY_TXT_KEY`.
  private static let relayKey = "relay"
  /// Bounded so a peer that answers the PTR but never resolves (asleep, moved
  /// off the network mid-query) releases its slot instead of pinning it for the
  /// life of the process.
  private static let resolveTimeout: TimeInterval = 5.0

  static let shared = OutlBonjour()

  /// Callback into Rust (`ios_bonjour::on_resolved`). A function pointer handed
  /// over at `start`, rather than a `@_silgen_name` symbol resolved at link
  /// time: a pointer passed at runtime cannot be dropped by dead-code
  /// elimination, and needs no guarantee about which crate survives the link.
  typealias ResolvedCallback = @convention(c) (
    UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafePointer<CChar>?
  ) -> Void

  private var callback: ResolvedCallback?

  private var browser: NetServiceBrowser?
  private var published: NetService?
  /// Strong refs while a resolve is in flight — `NetServiceBrowser` does not
  /// retain the services it hands out, and an unretained one is deallocated
  /// before its delegate ever fires.
  private var resolving: [String: NetService] = [:]

  // MARK: - Publish

  /// Advertise this endpoint under `endpointId`.
  ///
  /// Republishing (iroh re-publishes whenever its address set changes) stops
  /// the previous service first; two `NetService` objects on one instance name
  /// is a conflict `mDNSResponder` resolves by renaming ours to
  /// "`<id>` (2)", which no peer would then match to an endpoint id.
  func publish(endpointId: String, port: Int32, relay: String?) {
    published?.stop()
    published = nil
    guard port > 0, port <= 65535 else { return }

    let service = NetService(
      domain: Self.domain, type: Self.serviceType, name: endpointId, port: Int32(port))
    service.delegate = self

    var txt: [String: Data] = [:]
    if let relay, !relay.isEmpty, let encoded = relay.data(using: .utf8) {
      txt[Self.relayKey] = encoded
    }
    service.setTXTRecord(NetService.data(fromTXTRecord: txt))

    // `.listenForConnections` would make NetService open its own socket. We are
    // advertising a port iroh already owns, so publish the record only.
    service.publish()
    published = service
  }

  // MARK: - Browse

  func start(callback: @escaping ResolvedCallback) {
    self.callback = callback
    guard browser == nil else { return }
    let browser = NetServiceBrowser()
    browser.delegate = self
    // Bonjour's own run loop. Set before searching so the first answer, which
    // can arrive in the same turn on a quiet LAN, already has a delegate.
    browser.schedule(in: .main, forMode: .common)
    browser.searchForServices(ofType: Self.serviceType, inDomain: Self.domain)
    self.browser = browser
  }

  private func emit(_ service: NetService) {
    guard let callback else { return }

    // `addresses` is an array of `sockaddr` blobs. Render each as "ip:port" —
    // the same text `SocketAddr`'s `FromStr` parses on the Rust side, so the
    // conversion has one owner and it is the standard library's.
    let rendered = (service.addresses ?? []).compactMap(Self.describe).joined(separator: ",")
    guard !rendered.isEmpty else { return }

    var relay = ""
    if let data = service.txtRecordData() {
      let record = NetService.dictionary(fromTXTRecord: data)
      if let value = record[Self.relayKey], let text = String(data: value, encoding: .utf8) {
        relay = text
      }
    }

    service.name.withCString { idPtr in
      rendered.withCString { addrPtr in
        relay.withCString { relayPtr in
          callback(idPtr, addrPtr, relayPtr)
        }
      }
    }
  }

  /// One `sockaddr` blob to "ip:port", or nil for a family we cannot dial.
  private static func describe(_ data: Data) -> String? {
    data.withUnsafeBytes { raw -> String? in
      guard let base = raw.baseAddress, raw.count >= MemoryLayout<sockaddr>.size else {
        return nil
      }
      let family = base.assumingMemoryBound(to: sockaddr.self).pointee.sa_family
      var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
      var port = [CChar](repeating: 0, count: Int(NI_MAXSERV))
      // IPv4 only, matching the endpoint's IPv4-only bind on every platform.
      // `NI_NUMERICHOST` also renders a link-local v6 with its scope id
      // (`fe80::1%en0`), which Rust's `SocketAddr` parser rejects, so emitting
      // one would only move the discard one language further away.
      guard family == sa_family_t(AF_INET) else { return nil }
      // NI_NUMERICHOST keeps this off the network: no reverse DNS, no blocking
      // the main run loop on a resolver.
      guard
        getnameinfo(
          base.assumingMemoryBound(to: sockaddr.self), socklen_t(raw.count),
          &host, socklen_t(host.count), &port, socklen_t(port.count),
          NI_NUMERICHOST | NI_NUMERICSERV) == 0
      else { return nil }
      let ip = String(cString: host)
      let service = String(cString: port)
      guard !ip.isEmpty, !service.isEmpty, service != "0" else { return nil }
      return "\(ip):\(service)"
    }
  }
}

extension OutlBonjour: NetServiceBrowserDelegate {
  func netServiceBrowser(
    _ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool
  ) {
    // Our own advertisement comes back through the browser like any other.
    // Resolving it would hand iroh its own addresses as a peer's.
    guard service.name != published?.name else { return }
    service.delegate = self
    resolving[service.name] = service
    service.schedule(in: .main, forMode: .common)
    service.resolve(withTimeout: Self.resolveTimeout)
  }

  func netServiceBrowser(
    _ browser: NetServiceBrowser, didRemove service: NetService, moreComing: Bool
  ) {
    resolving.removeValue(forKey: service.name)
  }
}

extension OutlBonjour: NetServiceDelegate {
  func netServiceDidResolveAddress(_ service: NetService) {
    emit(service)
    resolving.removeValue(forKey: service.name)
  }

  func netService(_ service: NetService, didNotResolve error: [String: NSNumber]) {
    resolving.removeValue(forKey: service.name)
  }

  func netService(_ service: NetService, didNotPublish error: [String: NSNumber]) {
    if service === published { published = nil }
  }
}

// MARK: - C ABI consumed by `outl-sync-iroh`'s `bind::bonjour`

@_cdecl("outl_ios_bonjour_start")
public func outl_ios_bonjour_start(
  _ callback: @convention(c) (
    UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafePointer<CChar>?
  ) -> Void
) {
  DispatchQueue.main.async {
    OutlBonjour.shared.start(callback: callback)
  }
}

@_cdecl("outl_ios_bonjour_publish")
public func outl_ios_bonjour_publish(
  _ endpointId: UnsafePointer<CChar>?,
  _ port: UInt16,
  _ relay: UnsafePointer<CChar>?
) {
  guard let endpointId else { return }
  let id = String(cString: endpointId)
  let relayUrl = relay.map { String(cString: $0) }
  DispatchQueue.main.async {
    OutlBonjour.shared.publish(endpointId: id, port: Int32(port), relay: relayUrl)
  }
}
