//! Integration tests for flyq-network discovery and peer management.

use flyq_network::{DiscoveryConfig, DiscoveryEvent, DiscoveryService, PeerManager};
use flyq_protocol::{Command, PeerInfo, UserStatus};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::time::timeout;

// ─── PeerManager Tests ──────────────────────────────────────────────────────

fn make_peer(name: &str, ip: [u8; 4], port: u16) -> PeerInfo {
    PeerInfo {
        name: name.to_string(),
        host: format!("{}-PC", name.to_uppercase()),
        addr: IpAddr::V4(Ipv4Addr::from(ip)),
        port,
        online: true,
        group: None,
        mac: Some("AABBCCDDEEFF".to_string()),
        is_feiq: true,
        status: UserStatus::Online,
    }
}

#[tokio::test]
async fn test_peer_manager_add_and_get() {
    let pm = PeerManager::new();
    let peer = make_peer("Alice", [192, 168, 1, 10], 2425);

    pm.add_or_update_peer(peer.clone()).await;

    let found = pm.get_peer_by_addr(&peer.addr).await;
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Alice");
    assert_eq!(pm.peer_count().await, 1);
}

#[tokio::test]
async fn test_peer_manager_update_existing() {
    let pm = PeerManager::new();
    let mut peer = make_peer("Bob", [192, 168, 1, 20], 2425);
    pm.add_or_update_peer(peer.clone()).await;

    // Update name.
    peer.name = "Bob Updated".to_string();
    pm.add_or_update_peer(peer.clone()).await;

    assert_eq!(pm.peer_count().await, 1);
    let found = pm.get_peer_by_addr(&peer.addr).await.unwrap();
    assert_eq!(found.name, "Bob Updated");
}

#[tokio::test]
async fn test_peer_manager_remove() {
    let pm = PeerManager::new();
    let peer = make_peer("Carol", [192, 168, 1, 30], 2425);
    pm.add_or_update_peer(peer.clone()).await;

    let removed = pm.remove_peer_by_addr(&peer.addr).await;
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().name, "Carol");
    assert_eq!(pm.peer_count().await, 0);

    // Removing again returns None.
    assert!(pm.remove_peer_by_addr(&peer.addr).await.is_none());
}

#[tokio::test]
async fn test_peer_manager_online_peers() {
    let pm = PeerManager::new();

    let peer1 = make_peer("Dave", [10, 0, 0, 1], 2425);
    let mut peer2 = make_peer("Eve", [10, 0, 0, 2], 2425);
    peer2.online = false;
    peer2.status = UserStatus::Offline;

    pm.add_or_update_peer(peer1.clone()).await;
    pm.add_or_update_peer(peer2.clone()).await;

    let online = pm.online_peers().await;
    assert_eq!(online.len(), 1);
    assert_eq!(online[0].name, "Dave");

    let all = pm.all_peers().await;
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn test_peer_manager_status_change() {
    let pm = PeerManager::new();
    let peer = make_peer("Frank", [10, 0, 0, 3], 2425);
    pm.add_or_update_peer(peer.clone()).await;

    pm.set_peer_status(&peer.addr, UserStatus::Away).await;
    let updated = pm.get_peer_by_addr(&peer.addr).await.unwrap();
    assert_eq!(updated.status, UserStatus::Away);
    assert!(updated.online); // Away is still "online"

    pm.set_peer_status(&peer.addr, UserStatus::Offline).await;
    let updated = pm.get_peer_by_addr(&peer.addr).await.unwrap();
    assert_eq!(updated.status, UserStatus::Offline);
    assert!(!updated.online);
}

#[tokio::test]
async fn test_peer_manager_mark_offline() {
    let pm = PeerManager::new();
    let peer = make_peer("Grace", [10, 0, 0, 4], 2425);
    pm.add_or_update_peer(peer.clone()).await;

    let marked = pm.mark_peer_offline(&peer.addr).await;
    assert!(marked.is_some());
    assert!(!marked.unwrap().online);

    // Peer still exists in the map.
    assert_eq!(pm.peer_count().await, 1);
}

#[tokio::test]
async fn test_peer_manager_timeout() {
    let pm = PeerManager::new();
    let peer = make_peer("Heidi", [10, 0, 0, 5], 2425);
    pm.add_or_update_peer(peer.clone()).await;

    // With a very short timeout, the peer should be removed.
    // Use tokio::time::pause to control time in tests.
    // Since we can't easily pause time here, use Duration::ZERO
    // which means "already timed out".
    tokio::time::sleep(Duration::from_millis(10)).await;
    let removed = pm.remove_timed_out_peers(Duration::from_millis(5)).await;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].name, "Heidi");
    assert_eq!(pm.peer_count().await, 0);
}

#[tokio::test]
async fn test_peer_manager_touch_prevents_timeout() {
    let pm = PeerManager::new();
    let peer = make_peer("Ivan", [10, 0, 0, 6], 2425);
    pm.add_or_update_peer(peer.clone()).await;

    // Sleep briefly, then touch.
    tokio::time::sleep(Duration::from_millis(10)).await;
    pm.touch_peer(&peer.addr).await;

    // Timeout of 50ms should NOT remove the peer (just touched).
    let removed = pm.remove_timed_out_peers(Duration::from_millis(50)).await;
    assert_eq!(removed.len(), 0);
    assert_eq!(pm.peer_count().await, 1);
}

// ─── DiscoveryService Tests (UDP loopback) ──────────────────────────────────

#[tokio::test]
async fn test_discovery_service_creation() {
    // Bind on a random high port to avoid conflicts.
    let config = DiscoveryConfig {
        username: "TestUser".to_string(),
        hostname: "TEST-PC".to_string(),
        mac_address: "112233445566".to_string(),
        feiq_level: 128,
        port: 0, // Let OS assign a port.
        broadcast_interval: Duration::from_secs(3600), // Don't broadcast during test.
        peer_timeout: Duration::from_secs(3600),
        use_feiq_version: true,
        group_name: None,
    };

    // Port 0 won't work with our bind logic (we bind to 0.0.0.0:port).
    // Use a high port instead.
    let config = DiscoveryConfig {
        port: 19425,
        ..config
    };

    let service = DiscoveryService::new(config).await;
    assert!(service.is_ok(), "Failed to create discovery service: {:?}", service.err());
}

#[tokio::test]
async fn test_discovery_two_peers_loopback() {
    // Create two services on different ports communicating via loopback.
    let pm_a = PeerManager::new();
    let pm_b = PeerManager::new();

    let config_a = DiscoveryConfig {
        username: "Alice".to_string(),
        hostname: "ALICE-PC".to_string(),
        mac_address: "AAAAAAAAAAAA".to_string(),
        feiq_level: 128,
        port: 19501,
        broadcast_interval: Duration::from_secs(3600),
        peer_timeout: Duration::from_secs(3600),
        use_feiq_version: true,
        group_name: None,
    };

    let config_b = DiscoveryConfig {
        username: "Bob".to_string(),
        hostname: "BOB-PC".to_string(),
        mac_address: "BBBBBBBBBBBB".to_string(),
        feiq_level: 128,
        port: 19502,
        broadcast_interval: Duration::from_secs(3600),
        peer_timeout: Duration::from_secs(3600),
        use_feiq_version: true,
        group_name: None,
    };

    let service_a = DiscoveryService::new(config_a).await.unwrap();
    let service_b = DiscoveryService::new(config_b).await.unwrap();

    let shutdown_a = service_a.shutdown_handle();
    let shutdown_b = service_b.shutdown_handle();

    let (_rx_a, _handle_a) = service_a.spawn(pm_a.clone());
    let (mut rx_b, _handle_b) = service_b.spawn(pm_b.clone());

    // Give services time to start and send initial broadcast.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Service A broadcasts on 19501, B on 19502 — they won't see each other's
    // broadcasts directly (different ports). Simulate by sending a unicast
    // BrEntry from A to B's port.
    let br_entry_packet = flyq_protocol::PacketBuilder::new_feiq("AAAAAAAAAAAA", 128)
        .sender("Alice", "ALICE-PC")
        .packet_no(1000)
        .command(Command::BrEntry)
        .flag(flyq_protocol::command::flags::FEIQ_ONLINE_FLAGS)
        .build();

    let target_b = SocketAddr::from(([127, 0, 0, 1], 19502));
    // Use a temporary socket to send the packet.
    let tmp_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    tmp_sock.send_to(br_entry_packet.as_bytes(), target_b).await.unwrap();

    // Wait for B to process the packet.
    let event = timeout(Duration::from_secs(2), rx_b.recv()).await;
    assert!(event.is_ok(), "Timeout waiting for PeerJoined event on B");

    let event = event.unwrap().unwrap();
    match event {
        DiscoveryEvent::PeerJoined(peer) => {
            assert_eq!(peer.name, "Alice");
            assert_eq!(peer.host, "ALICE-PC");
            assert!(peer.is_feiq);
            assert_eq!(peer.mac.as_deref(), Some("AAAAAAAAAAAA"));
        }
        other => panic!("Expected PeerJoined, got {:?}", other),
    }

    // B should have responded with AnsEntry back to the sender.
    // Check that A's peer manager eventually gets Bob (via AnsEntry response).
    // The AnsEntry was sent to 127.0.0.1:tmp_port, not to A's service port,
    // so A won't see it. Instead, verify B's peer manager has Alice.
    assert_eq!(pm_b.peer_count().await, 1);
    let alice = pm_b.get_peer_by_addr(&IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
    assert!(alice.is_some());

    // Shutdown both services.
    shutdown_a.shutdown();
    shutdown_b.shutdown();
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_discovery_br_exit_removes_peer() {
    let pm = PeerManager::new();

    let config = DiscoveryConfig {
        username: "Listener".to_string(),
        hostname: "LISTEN-PC".to_string(),
        mac_address: "CCCCCCCCCCCC".to_string(),
        feiq_level: 128,
        port: 19503,
        broadcast_interval: Duration::from_secs(3600),
        peer_timeout: Duration::from_secs(3600),
        use_feiq_version: true,
        group_name: None,
    };

    let service = DiscoveryService::new(config).await.unwrap();
    let shutdown = service.shutdown_handle();
    let (mut rx, _handle) = service.spawn(pm.clone());

    tokio::time::sleep(Duration::from_millis(50)).await;

    let tmp_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = SocketAddr::from(([127, 0, 0, 1], 19503));

    // Send BrEntry first.
    let entry = flyq_protocol::PacketBuilder::new_feiq("DDDDDDDDDDDD", 128)
        .sender("Departing", "DEPART-PC")
        .packet_no(2000)
        .command(Command::BrEntry)
        .flag(flyq_protocol::command::flags::FEIQ_ONLINE_FLAGS)
        .build();
    tmp_sock.send_to(entry.as_bytes(), target).await.unwrap();

    // Wait for PeerJoined.
    let event = timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
    match &event {
        DiscoveryEvent::PeerJoined(p) => assert_eq!(p.name, "Departing"),
        other => panic!("Expected PeerJoined, got {:?}", other),
    }
    assert_eq!(pm.peer_count().await, 1);

    // Now send BrExit from the same address.
    // Note: BrExit must come from the same IP for removal to work.
    let exit = flyq_protocol::PacketBuilder::new_feiq("DDDDDDDDDDDD", 128)
        .sender("Departing", "DEPART-PC")
        .packet_no(2001)
        .command(Command::BrExit)
        .build();
    tmp_sock.send_to(exit.as_bytes(), target).await.unwrap();

    // Wait for PeerLeft.
    let event = timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
    match &event {
        DiscoveryEvent::PeerLeft(p) => assert_eq!(p.name, "Departing"),
        other => panic!("Expected PeerLeft, got {:?}", other),
    }
    assert_eq!(pm.peer_count().await, 0);

    shutdown.shutdown();
}

#[tokio::test]
async fn test_discovery_message_forwarding() {
    let pm = PeerManager::new();

    let config = DiscoveryConfig {
        username: "Receiver".to_string(),
        hostname: "RECV-PC".to_string(),
        mac_address: "EEEEEEEEEEEE".to_string(),
        feiq_level: 128,
        port: 19504,
        broadcast_interval: Duration::from_secs(3600),
        peer_timeout: Duration::from_secs(3600),
        use_feiq_version: true,
        group_name: None,
    };

    let service = DiscoveryService::new(config).await.unwrap();
    let shutdown = service.shutdown_handle();
    let (mut rx, _handle) = service.spawn(pm.clone());

    tokio::time::sleep(Duration::from_millis(50)).await;

    let tmp_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = SocketAddr::from(([127, 0, 0, 1], 19504));

    // Send a chat message (SendMsg command).
    let msg_packet = flyq_protocol::PacketBuilder::new_feiq("FFFFFFFFFFFF", 128)
        .sender("Sender", "SEND-PC")
        .packet_no(3000)
        .command(Command::SendMsg)
        .extra("Hello World!")
        .build();
    tmp_sock.send_to(msg_packet.as_bytes(), target).await.unwrap();

    // Should be forwarded as PacketReceived event.
    let event = timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
    match event {
        DiscoveryEvent::PacketReceived { packet, from } => {
            assert_eq!(packet.command, Command::SendMsg);
            assert_eq!(packet.extra.as_deref(), Some("Hello World!"));
            assert_eq!(packet.sender_name, "Sender");
            assert_eq!(from.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        }
        other => panic!("Expected PacketReceived, got {:?}", other),
    }

    shutdown.shutdown();
}

#[tokio::test]
async fn test_discovery_typing_indicator() {
    let pm = PeerManager::new();

    let config = DiscoveryConfig {
        username: "Watcher".to_string(),
        hostname: "WATCH-PC".to_string(),
        mac_address: "123456789ABC".to_string(),
        feiq_level: 128,
        port: 19505,
        broadcast_interval: Duration::from_secs(3600),
        peer_timeout: Duration::from_secs(3600),
        use_feiq_version: true,
        group_name: None,
    };

    let service = DiscoveryService::new(config).await.unwrap();
    let shutdown = service.shutdown_handle();
    let (mut rx, _handle) = service.spawn(pm.clone());

    tokio::time::sleep(Duration::from_millis(50)).await;

    let tmp_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = SocketAddr::from(([127, 0, 0, 1], 19505));

    // Send typing indicator.
    let typing = flyq_protocol::PacketBuilder::new_feiq("ABCDEF123456", 128)
        .sender("Typer", "TYPE-PC")
        .packet_no(4000)
        .command(Command::TypingStart)
        .build();
    tmp_sock.send_to(typing.as_bytes(), target).await.unwrap();

    let event = timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
    match event {
        DiscoveryEvent::PacketReceived { packet, .. } => {
            assert_eq!(packet.command, Command::TypingStart);
            assert_eq!(packet.sender_name, "Typer");
        }
        other => panic!("Expected PacketReceived for typing, got {:?}", other),
    }

    shutdown.shutdown();
}

#[tokio::test]
async fn test_discovery_graceful_shutdown() {
    let pm = PeerManager::new();

    let config = DiscoveryConfig {
        username: "ShutdownTest".to_string(),
        hostname: "SHUT-PC".to_string(),
        mac_address: "999999999999".to_string(),
        feiq_level: 128,
        port: 19506,
        broadcast_interval: Duration::from_secs(3600),
        peer_timeout: Duration::from_secs(3600),
        use_feiq_version: true,
        group_name: None,
    };

    let service = DiscoveryService::new(config).await.unwrap();
    let shutdown = service.shutdown_handle();
    let (_rx, handle) = service.spawn(pm);

    // Service should be running.
    assert!(!handle.is_finished());

    // Trigger shutdown.
    shutdown.shutdown();

    // Wait for the task to finish.
    let result = timeout(Duration::from_secs(2), handle).await;
    assert!(result.is_ok(), "Discovery service did not shut down in time");
    result.unwrap().unwrap(); // Should not panic.
}

// ─── Broadcast Address Discovery Test ───────────────────────────────────────

#[tokio::test]
async fn test_discovery_config_default() {
    let config = DiscoveryConfig::default();
    assert_eq!(config.port, 2425);
    assert!(config.use_feiq_version);
    assert_eq!(config.feiq_level, 128);
    assert!(!config.mac_address.is_empty());
    assert_eq!(config.mac_address.len(), 12); // 12 hex chars
    assert_eq!(config.broadcast_interval, Duration::from_secs(60));
    assert_eq!(config.peer_timeout, Duration::from_secs(180));
}
