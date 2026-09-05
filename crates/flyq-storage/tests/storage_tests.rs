//! Integration tests for flyq-storage.

use flyq_storage::{Database, Group, Page, StoredMessage, StoredPeer};
use uuid::Uuid;

/// Create an in-memory database for testing.
async fn setup_db() -> Database {
    let db = Database::open(":memory:").await.unwrap();
    db.migrate().await.unwrap();
    db
}

/// Helper to create a test message.
fn make_msg(sender: &str, recipient: &str, content: &str, timestamp: i64, read: bool) -> StoredMessage {
    StoredMessage {
        id: Uuid::new_v4().to_string(),
        sender: sender.to_string(),
        recipient: recipient.to_string(),
        content: content.to_string(),
        timestamp,
        read,
        packet_no: None,
        group_id: None,
        media_type: 0,
    }
}

// ─── Basic CRUD Tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_insert_and_get_messages() {
    let db = setup_db().await;

    let msg1 = make_msg("192.168.1.1:2425", "192.168.1.2:2425", "Hello!", 1000, false);
    let msg2 = make_msg("192.168.1.2:2425", "192.168.1.1:2425", "Hi there!", 1001, false);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();

    let messages = db
        .get_messages("192.168.1.1:2425", "192.168.1.2:2425", 10)
        .await
        .unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "Hello!");
    assert_eq!(messages[1].content, "Hi there!");
}

#[tokio::test]
async fn test_delete_message() {
    let db = setup_db().await;

    let msg = make_msg("a", "b", "delete me", 1000, false);
    db.insert_message(&msg).await.unwrap();

    let deleted = db.delete_message(&msg.id).await.unwrap();
    assert!(deleted);

    let not_found = db.delete_message("nonexistent").await.unwrap();
    assert!(!not_found);

    let messages = db.get_messages("a", "b", 10).await.unwrap();
    assert!(messages.is_empty());
}

#[tokio::test]
async fn test_mark_message_read() {
    let db = setup_db().await;

    let msg = make_msg("a", "b", "unread msg", 1000, false);
    db.insert_message(&msg).await.unwrap();

    db.mark_message_read(&msg.id).await.unwrap();

    let messages = db.get_messages("a", "b", 10).await.unwrap();
    assert!(messages[0].read);
}

#[tokio::test]
async fn test_mark_conversation_read() {
    let db = setup_db().await;

    let msg1 = make_msg("peer1", "me", "msg1", 1000, false);
    let msg2 = make_msg("peer1", "me", "msg2", 1001, false);
    let msg3 = make_msg("me", "peer1", "my reply", 1002, true);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();
    db.insert_message(&msg3).await.unwrap();

    let count = db.mark_conversation_read("me", "peer1").await.unwrap();
    assert_eq!(count, 2);

    let unread = db.get_unread_count("me").await.unwrap();
    assert_eq!(unread, 0);
}

// ─── Pagination Tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_paginated_messages() {
    let db = setup_db().await;

    // Insert 25 messages.
    for i in 0..25 {
        let msg = make_msg("a", "b", &format!("message {}", i), 1000 + i as i64, false);
        db.insert_message(&msg).await.unwrap();
    }

    // First page: 10 items.
    let page1 = db
        .get_messages_paged("a", "b", Page::new(10, 0))
        .await
        .unwrap();
    assert_eq!(page1.items.len(), 10);
    assert_eq!(page1.total, 25);
    assert!(page1.has_more());
    assert_eq!(page1.total_pages(), 3);
    // Should be chronological within page (oldest first).
    assert_eq!(page1.items[0].content, "message 15");
    assert_eq!(page1.items[9].content, "message 24");

    // Second page.
    let page2 = db
        .get_messages_paged("a", "b", Page::new(10, 10))
        .await
        .unwrap();
    assert_eq!(page2.items.len(), 10);
    assert!(page2.has_more());

    // Third page (last, partial).
    let page3 = db
        .get_messages_paged("a", "b", Page::new(10, 20))
        .await
        .unwrap();
    assert_eq!(page3.items.len(), 5);
    assert!(!page3.has_more());
}

#[tokio::test]
async fn test_page_from_page_number() {
    let page = Page::from_page_number(3, 20);
    assert_eq!(page.limit, 20);
    assert_eq!(page.offset, 40);

    // Page 1 should have offset 0.
    let page1 = Page::from_page_number(1, 20);
    assert_eq!(page1.offset, 0);

    // Page 0 should also have offset 0 (saturating_sub).
    let page0 = Page::from_page_number(0, 20);
    assert_eq!(page0.offset, 0);
}

#[tokio::test]
async fn test_messages_by_time_range() {
    let db = setup_db().await;

    for i in 0..20 {
        let msg = make_msg("a", "b", &format!("msg at {}", 1000 + i * 100), 1000 + i * 100, false);
        db.insert_message(&msg).await.unwrap();
    }

    // Get messages in range [1500, 1900].
    let result = db
        .get_messages_by_time_range("a", "b", 1500, 1900, Page::new(50, 0))
        .await
        .unwrap();

    assert_eq!(result.total, 5); // timestamps 1500, 1600, 1700, 1800, 1900
    assert_eq!(result.items.len(), 5);
    // Should be in ascending timestamp order.
    assert!(result.items[0].timestamp <= result.items[1].timestamp);
}

// ─── Full-Text Search Tests ─────────────────────────────────────────────────

#[tokio::test]
async fn test_search_messages_basic() {
    let db = setup_db().await;

    let msg1 = make_msg("a", "b", "The quick brown fox jumps", 1000, false);
    let msg2 = make_msg("a", "b", "A lazy dog sleeps all day", 1001, false);
    let msg3 = make_msg("b", "a", "Fox hunting is popular", 1002, false);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();
    db.insert_message(&msg3).await.unwrap();

    // Search for "fox".
    let results = db.search_messages("fox", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 2);
    assert_eq!(results.items.len(), 2);

    // Search for "dog".
    let results = db.search_messages("dog", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 1);
    assert!(results.items[0].message.content.contains("dog"));
}

#[tokio::test]
async fn test_search_messages_chinese() {
    let db = setup_db().await;

    // NOTE: FTS5 unicode61 tokenizer treats continuous CJK characters as a single token.
    // Punctuation (，。！？、) acts as separators, creating distinct searchable tokens.
    // FTS5 matches WHOLE tokens only — "会议" won't match token "有个会议".
    // This matches real Chinese chat behavior where users naturally use punctuation.
    let msg1 = make_msg("a", "b", "今天，天气，很好", 1000, false);
    let msg2 = make_msg("a", "b", "明天，会下雨吗", 1001, false);
    let msg3 = make_msg("b", "a", "今天，会议，下午三点", 1002, false);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();
    db.insert_message(&msg3).await.unwrap();

    // "今天" is a standalone token in msg1 and msg3.
    let results = db.search_messages("今天", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 2);

    // "天气" is a standalone token in msg1 (separated by commas).
    let results = db.search_messages("天气", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 1);

    // "会议" is a standalone token in msg3.
    let results = db.search_messages("会议", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 1);

    // "明天" is a standalone token in msg2.
    let results = db.search_messages("明天", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 1);

    // "会下雨吗" is one token (no internal punctuation) in msg2.
    let results = db.search_messages("会下雨吗", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 1);

    // Substring "下雨" does NOT match token "会下雨吗" (exact token matching).
    let results = db.search_messages("下雨", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 0);
}

#[tokio::test]
async fn test_search_with_snippet() {
    let db = setup_db().await;

    let long_content = "This is a very long message that contains the keyword somewhere in the middle of all this text and we want to see it highlighted";
    let msg = make_msg("a", "b", long_content, 1000, false);
    db.insert_message(&msg).await.unwrap();

    let results = db
        .search_messages("keyword", Page::new(10, 0))
        .await
        .unwrap();
    assert_eq!(results.total, 1);
    // Snippet should contain highlight markers.
    assert!(results.items[0].snippet.contains("<<"));
    assert!(results.items[0].snippet.contains(">>"));
}

#[tokio::test]
async fn test_search_in_conversation() {
    let db = setup_db().await;

    let msg1 = make_msg("a", "b", "meeting at 3pm", 1000, false);
    let msg2 = make_msg("a", "c", "meeting cancelled", 1001, false);
    let msg3 = make_msg("b", "a", "ok see you at the meeting", 1002, false);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();
    db.insert_message(&msg3).await.unwrap();

    // Search "meeting" only in conversation between a and b.
    let results = db
        .search_in_conversation("a", "b", "meeting", Page::new(10, 0))
        .await
        .unwrap();
    assert_eq!(results.total, 2);

    // Search "meeting" in conversation between a and c.
    let results = db
        .search_in_conversation("a", "c", "meeting", Page::new(10, 0))
        .await
        .unwrap();
    assert_eq!(results.total, 1);
}

#[tokio::test]
async fn test_search_pagination() {
    let db = setup_db().await;

    // Insert 15 messages all containing "test".
    for i in 0..15 {
        let msg = make_msg("a", "b", &format!("test message number {}", i), 1000 + i as i64, false);
        db.insert_message(&msg).await.unwrap();
    }

    let page1 = db.search_messages("test", Page::new(5, 0)).await.unwrap();
    assert_eq!(page1.items.len(), 5);
    assert_eq!(page1.total, 15);
    assert!(page1.has_more());

    let page3 = db.search_messages("test", Page::new(5, 10)).await.unwrap();
    assert_eq!(page3.items.len(), 5);
    assert!(!page3.has_more());
}

// ─── Conversation Summary Tests ─────────────────────────────────────────────

#[tokio::test]
async fn test_get_conversations() {
    let db = setup_db().await;

    // Set up peers.
    let peer1 = StoredPeer {
        addr: "peer1".to_string(),
        name: "Alice".to_string(),
        host: "alice-pc".to_string(),
        group: Some("dev".to_string()),
        last_seen: 1000,
    };
    let peer2 = StoredPeer {
        addr: "peer2".to_string(),
        name: "Bob".to_string(),
        host: "bob-pc".to_string(),
        group: None,
        last_seen: 1001,
    };
    db.upsert_peer(&peer1).await.unwrap();
    db.upsert_peer(&peer2).await.unwrap();

    // Insert messages.
    let msg1 = make_msg("me", "peer1", "Hey Alice", 1000, true);
    let msg2 = make_msg("peer1", "me", "Hi! How are you?", 1001, false);
    let msg3 = make_msg("me", "peer2", "Bob, check this", 1002, true);
    let msg4 = make_msg("peer2", "me", "Sure, sending now", 1003, false);
    let msg5 = make_msg("peer1", "me", "Are you there?", 1004, false);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();
    db.insert_message(&msg3).await.unwrap();
    db.insert_message(&msg4).await.unwrap();
    db.insert_message(&msg5).await.unwrap();

    let conversations = db.get_conversations("me").await.unwrap();
    assert_eq!(conversations.len(), 2);

    // Most recent conversation first (peer1, timestamp 1004).
    assert_eq!(conversations[0].peer_addr, "peer1");
    assert_eq!(conversations[0].peer_name, "Alice");
    assert_eq!(conversations[0].last_message, "Are you there?");
    assert_eq!(conversations[0].unread_count, 2);

    assert_eq!(conversations[1].peer_addr, "peer2");
    assert_eq!(conversations[1].peer_name, "Bob");
    assert_eq!(conversations[1].unread_count, 1);
}

#[tokio::test]
async fn test_message_stats() {
    let db = setup_db().await;

    let msg1 = make_msg("a", "b", "hello", 1000, true);
    let msg2 = make_msg("b", "a", "hi", 1001, false);
    let msg3 = make_msg("a", "b", "how are you", 1002, true);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();
    db.insert_message(&msg3).await.unwrap();

    let stats = db.get_message_stats("a", "b").await.unwrap();
    assert_eq!(stats.total, 3);
    assert_eq!(stats.unread, 1); // msg2 is unread from b to a
    assert_eq!(stats.first_timestamp, Some(1000));
    assert_eq!(stats.last_timestamp, Some(1002));
}

#[tokio::test]
async fn test_unread_count() {
    let db = setup_db().await;

    let msg1 = make_msg("peer1", "me", "unread 1", 1000, false);
    let msg2 = make_msg("peer2", "me", "unread 2", 1001, false);
    let msg3 = make_msg("me", "peer1", "my sent msg", 1002, true);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();
    db.insert_message(&msg3).await.unwrap();

    let count = db.get_unread_count("me").await.unwrap();
    assert_eq!(count, 2);
}

// ─── Peer Management Tests ──────────────────────────────────────────────────

#[tokio::test]
async fn test_peer_crud() {
    let db = setup_db().await;

    let peer = StoredPeer {
        addr: "192.168.1.100:2425".to_string(),
        name: "TestUser".to_string(),
        host: "test-host".to_string(),
        group: Some("engineering".to_string()),
        last_seen: 1000,
    };

    db.upsert_peer(&peer).await.unwrap();

    let peers = db.get_peers().await.unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].name, "TestUser");
    assert_eq!(peers[0].group, Some("engineering".to_string()));

    // Update peer.
    let updated = StoredPeer {
        name: "TestUser2".to_string(),
        last_seen: 2000,
        ..peer.clone()
    };
    db.upsert_peer(&updated).await.unwrap();

    let peers = db.get_peers().await.unwrap();
    assert_eq!(peers[0].name, "TestUser2");
    assert_eq!(peers[0].last_seen, 2000);

    // Touch peer.
    db.touch_peer("192.168.1.100:2425", 3000).await.unwrap();
    let peers = db.get_peers().await.unwrap();
    assert_eq!(peers[0].last_seen, 3000);

    // Delete peer.
    let deleted = db.delete_peer("192.168.1.100:2425").await.unwrap();
    assert!(deleted);
    let peers = db.get_peers().await.unwrap();
    assert!(peers.is_empty());
}

// ─── FTS Index Rebuild Test ─────────────────────────────────────────────────

#[tokio::test]
async fn test_rebuild_fts_index() {
    let db = setup_db().await;

    let msg = make_msg("a", "b", "rebuild test content", 1000, false);
    db.insert_message(&msg).await.unwrap();

    // Verify search works before rebuild.
    let results = db.search_messages("rebuild", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 1);

    // Rebuild index.
    db.rebuild_fts_index().await.unwrap();

    // Verify search still works after rebuild.
    let results = db.search_messages("rebuild", Page::new(10, 0)).await.unwrap();
    assert_eq!(results.total, 1);
}

// ─── Edge Cases ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_empty_database_queries() {
    let db = setup_db().await;

    let messages = db.get_messages("nobody", "nobody2", 10).await.unwrap();
    assert!(messages.is_empty());

    let paged = db
        .get_messages_paged("nobody", "nobody2", Page::default())
        .await
        .unwrap();
    assert_eq!(paged.total, 0);
    assert!(!paged.has_more());

    let conversations = db.get_conversations("nobody").await.unwrap();
    assert!(conversations.is_empty());

    let unread = db.get_unread_count("nobody").await.unwrap();
    assert_eq!(unread, 0);

    let stats = db.get_message_stats("nobody", "nobody2").await.unwrap();
    assert_eq!(stats.total, 0);
}

#[tokio::test]
async fn test_messages_by_user() {
    let db = setup_db().await;

    let msg1 = make_msg("me", "peer1", "to peer1", 1000, true);
    let msg2 = make_msg("peer1", "me", "from peer1", 1001, false);
    let msg3 = make_msg("me", "peer2", "to peer2", 1002, true);
    let msg4 = make_msg("peer3", "peer4", "unrelated", 1003, false);

    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();
    db.insert_message(&msg3).await.unwrap();
    db.insert_message(&msg4).await.unwrap();

    let result = db
        .get_messages_by_user("me", Page::new(10, 0))
        .await
        .unwrap();
    assert_eq!(result.total, 3); // msg1, msg2, msg3 involve "me"
}

// ─── M6: DelMsg / packet_no Tests ──────────────────────────────────────────

#[tokio::test]
async fn test_packet_no_stored_and_retrieved() {
    let db = setup_db().await;

    let mut msg = make_msg("10.0.0.1:2425", "10.0.0.2:2425", "with pkt", 5000, false);
    msg.packet_no = Some(42);
    db.insert_message(&msg).await.unwrap();

    let messages = db
        .get_messages("10.0.0.1:2425", "10.0.0.2:2425", 10)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].packet_no, Some(42));
}

#[tokio::test]
async fn test_delete_message_by_packet_no() {
    let db = setup_db().await;

    let mut msg1 = make_msg("10.0.0.1:2425", "10.0.0.2:2425", "first", 6000, false);
    msg1.packet_no = Some(100);
    let mut msg2 = make_msg("10.0.0.1:2425", "10.0.0.2:2425", "second", 6001, false);
    msg2.packet_no = Some(101);
    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();

    // Delete by packet_no should remove only the matching message.
    let deleted = db
        .delete_message_by_packet_no("10.0.0.1:2425", 100)
        .await
        .unwrap();
    assert!(deleted, "should have deleted one message");

    let remaining = db
        .get_messages("10.0.0.1:2425", "10.0.0.2:2425", 10)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].packet_no, Some(101));
}

#[tokio::test]
async fn test_delete_message_by_packet_no_not_found() {
    let db = setup_db().await;

    let msg = make_msg("10.0.0.1:2425", "10.0.0.2:2425", "only", 7000, false);
    db.insert_message(&msg).await.unwrap();

    // Trying to delete with a non-existent packet_no should return false.
    let deleted = db
        .delete_message_by_packet_no("10.0.0.1:2425", 999)
        .await
        .unwrap();
    assert!(!deleted, "no message with that packet_no");

    // Also verify wrong sender_addr doesn't match.
    let deleted = db
        .delete_message_by_packet_no("10.0.0.99:2425", 999)
        .await
        .unwrap();
    assert!(!deleted, "wrong sender address");
}

// ─── M7: Group Management & Group Messages ─────────────────────────────────

fn make_group(id: &str, name: &str, members: Vec<&str>) -> Group {
    Group {
        id: id.to_string(),
        name: name.to_string(),
        members: members.into_iter().map(|s| s.to_string()).collect(),
        created_at: 1000,
    }
}

fn make_group_msg(sender: &str, group_id: &str, content: &str, timestamp: i64) -> StoredMessage {
    StoredMessage {
        id: Uuid::new_v4().to_string(),
        sender: sender.to_string(),
        recipient: String::new(),
        content: content.to_string(),
        timestamp,
        read: false,
        packet_no: None,
        group_id: Some(group_id.to_string()),
        media_type: 0,
    }
}

#[tokio::test]
async fn test_group_crud() {
    let db = setup_db().await;

    let group = make_group("g1", "开发团队", vec!["10.0.0.1:2425", "10.0.0.2:2425", "10.0.0.3:2425"]);
    db.insert_group(&group).await.unwrap();

    // Get all groups.
    let groups = db.get_groups().await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "开发团队");
    assert_eq!(groups[0].members.len(), 3);

    // Find by name.
    let found = db.get_group_by_name("开发团队").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, "g1");

    // Find non-existent name.
    let not_found = db.get_group_by_name("不存在的群").await.unwrap();
    assert!(not_found.is_none());

    // Update members.
    db.update_group_members("g1", &["10.0.0.1:2425".to_string(), "10.0.0.4:2425".to_string()])
        .await
        .unwrap();
    let groups = db.get_groups().await.unwrap();
    assert_eq!(groups[0].members.len(), 2);

    // Delete.
    let deleted = db.delete_group("g1").await.unwrap();
    assert!(deleted);
    let groups = db.get_groups().await.unwrap();
    assert!(groups.is_empty());
}

#[tokio::test]
async fn test_group_messages_paged() {
    let db = setup_db().await;

    let group = make_group("g1", "测试群", vec!["a", "b", "c"]);
    db.insert_group(&group).await.unwrap();

    // Insert 5 group messages from different senders.
    for i in 0..5 {
        let sender = if i % 2 == 0 { "a" } else { "b" };
        let msg = make_group_msg(sender, "g1", &format!("群消息 {}", i), 2000 + i as i64);
        db.insert_message(&msg).await.unwrap();
    }

    // Also insert a 1:1 message to verify it doesn't leak into group results.
    let direct = make_msg("a", "b", "私聊消息", 2500, false);
    db.insert_message(&direct).await.unwrap();

    let page = db.get_group_messages_paged("g1", Page::new(10, 0)).await.unwrap();
    assert_eq!(page.total, 5);
    assert_eq!(page.items.len(), 5);
    // Should be in chronological order.
    assert!(page.items[0].timestamp <= page.items[1].timestamp);

    // Pagination: first page of 3.
    let page1 = db.get_group_messages_paged("g1", Page::new(3, 0)).await.unwrap();
    assert_eq!(page1.items.len(), 3);
    assert!(page1.has_more());

    // Second page.
    let page2 = db.get_group_messages_paged("g1", Page::new(3, 3)).await.unwrap();
    assert_eq!(page2.items.len(), 2);
    assert!(!page2.has_more());
}

#[tokio::test]
async fn test_group_conversation_read() {
    let db = setup_db().await;

    let msg1 = make_group_msg("a", "g1", "消息1", 3000);
    let msg2 = make_group_msg("b", "g1", "消息2", 3001);
    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();

    // Both should be unread.
    let affected = db.mark_group_conversation_read("g1").await.unwrap();
    assert_eq!(affected, 2);

    // Mark again — no change.
    let affected = db.mark_group_conversation_read("g1").await.unwrap();
    assert_eq!(affected, 0);
}

#[tokio::test]
async fn test_group_summaries() {
    let db = setup_db().await;

    let g1 = make_group("g1", "群A", vec!["a", "b"]);
    let g2 = make_group("g2", "群B", vec!["a", "c", "d"]);
    db.insert_group(&g1).await.unwrap();
    db.insert_group(&g2).await.unwrap();

    // Group 1: 2 messages.
    let msg1 = make_group_msg("a", "g1", "群A消息1", 4000);
    let msg2 = make_group_msg("b", "g1", "群A消息2", 4001);
    db.insert_message(&msg1).await.unwrap();
    db.insert_message(&msg2).await.unwrap();

    // Group 2: 1 message.
    let msg3 = make_group_msg("c", "g2", "群B消息1", 4002);
    db.insert_message(&msg3).await.unwrap();

    let summaries = db.get_group_summaries().await.unwrap();
    assert_eq!(summaries.len(), 2);

    // Most recent first.
    assert_eq!(summaries[0].group_name, "群B");
    assert_eq!(summaries[0].last_message, "群B消息1");
    assert_eq!(summaries[0].unread_count, 1);
    assert_eq!(summaries[0].member_count, 3);

    assert_eq!(summaries[1].group_name, "群A");
    assert_eq!(summaries[1].unread_count, 2);
    assert_eq!(summaries[1].member_count, 2);
}

#[tokio::test]
async fn test_group_id_does_not_affect_direct_messages() {
    let db = setup_db().await;

    // Insert a direct message (no group_id).
    let direct = make_msg("a", "b", "私聊", 5000, false);
    db.insert_message(&direct).await.unwrap();

    // Insert a group message.
    let group_msg = make_group_msg("a", "g1", "群消息", 5001);
    db.insert_message(&group_msg).await.unwrap();

    // 1:1 conversation should only have the direct message.
    let messages = db.get_messages("a", "b", 10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "私聊");
    assert!(messages[0].group_id.is_none());

    // Group conversation should only have the group message.
    let group_msgs = db.get_group_messages_paged("g1", Page::new(10, 0)).await.unwrap();
    assert_eq!(group_msgs.total, 1);
    assert_eq!(group_msgs.items[0].content, "群消息");
    assert_eq!(group_msgs.items[0].group_id.as_deref(), Some("g1"));
}
