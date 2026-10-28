//
// Copyright (c) 2026 ZettaScale Technology
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   ZettaScale Zenoh Team, <zenoh@zettascale.tech>
//
use std::collections::HashMap;

use zenoh_core::{zread, zwrite};
use zenoh_protocol::{
    core::{Bound, ExprId, Region, WhatAmI, EMPTY_EXPR_ID},
    network::{
        interest::{InterestMode, InterestOptions},
        Declare, DeclareBody,
    },
};
use zenoh_sync::get_mut_unchecked;

use super::{try_init_tracing_subscriber, FaceDef, Harness, HarnessBuilder, Message, MockFace};
use crate::net::primitives::Primitives;

#[test]
fn routed_messages_survive_sender_mapping_reclamation_before_enqueue() {
    use std::sync::{Arc, Mutex};

    use zenoh_protocol::{
        core::Reliability,
        network::{Interest, Push, Request, Response, ResponseFinal},
    };

    use crate::net::{primitives::EPrimitives, routing::RoutingContext};

    struct RetireBeforeEnqueue {
        original: Arc<dyn EPrimitives + Send + Sync>,
        retire: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }
    impl RetireBeforeEnqueue {
        fn retire(&self) {
            if let Some(retire) = self.retire.lock().unwrap().take() {
                retire();
            }
        }
    }
    impl EPrimitives for RetireBeforeEnqueue {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn send_interest(&self, ctx: RoutingContext<&mut Interest>) -> bool {
            self.original.send_interest(ctx)
        }
        fn send_declare(&self, ctx: RoutingContext<&mut Declare>) -> bool {
            self.original.send_declare(ctx)
        }
        fn send_push(&self, msg: &mut Push, reliability: Reliability) -> bool {
            self.retire();
            self.original.send_push(msg, reliability)
        }
        fn send_request(&self, msg: &mut Request) -> bool {
            self.retire();
            self.original.send_request(msg)
        }
        fn send_response(&self, msg: &mut Response) -> bool {
            self.original.send_response(msg)
        }
        fn send_response_final(&self, msg: &mut ResponseFinal) -> bool {
            self.original.send_response_final(msg)
        }
    }

    for query in [false, true] {
        let (_peer, router, session) = peer_with_router_face();
        let session = Arc::new(session);
        router.declare_subscriber(None, 1, "queued/**");
        router.declare_queryable(None, 2, "queued/**", Default::default());
        session.declare_token(None, 1, "queued/topic");
        assert_eq!(local_mappings(&router), 1);

        // The route has already selected its WireExpr and released the table
        // lock when EPrimitives is called. Retire the only exact-key entity
        // before enqueue, while the wildcard endpoints remain declared.
        let original = {
            let _tables = zwrite!(router.face.tables.tables);
            let mut state = router.face.state.clone();
            let state = get_mut_unchecked(&mut state);
            let original = state.primitives.clone();
            let retire_session = session.clone();
            state.primitives = Arc::new(RetireBeforeEnqueue {
                original: original.clone(),
                retire: Mutex::new(Some(Box::new(move || retire_session.undeclare_token(1)))),
            });
            original
        };
        if query {
            session.query(1, "queued/topic");
        } else {
            session.put("queued/topic", vec![42]);
        }
        {
            let _tables = zwrite!(router.face.tables.tables);
            get_mut_unchecked(&mut router.face.state.clone()).primitives = original;
        }
        assert_eq!(local_mappings(&router), 0, "cleanup must actually run");
        let wire = if query {
            let requests = router.recorder().requests();
            assert_eq!(requests.len(), 1);
            requests[0].wire_expr.clone()
        } else {
            let pushes = router.recorder().pushes();
            assert_eq!(pushes.len(), 1);
            pushes[0].wire_expr.clone()
        };
        assert_eq!(
            wire.scope, EMPTY_EXPR_ID,
            "queued message uses a retired sender mapping"
        );
        assert_eq!(wire.suffix, "queued/topic");
    }
}

/// A peer gateway with one local session and one face towards a router, the shape of a
/// ROS 2 process running rmw_zenoh in peer mode next to `rmw_zenohd`.
fn peer_with_router_face() -> (Harness, MockFace, MockFace) {
    try_init_tracing_subscriber();
    let peer = HarnessBuilder::new()
        .mode(WhatAmI::Peer)
        .subregions([Region::Local])
        .build();
    let router = peer.new_face(
        FaceDef::default()
            .mode(WhatAmI::Router)
            .remote_bound(Bound::South),
    );
    let session = peer.new_session();
    (peer, router, session)
}

/// Take every remaining expr id of `face`, as if 65 535 key expressions were mapped on it.
fn exhaust_expr_ids(face: &MockFace) {
    let mut state = face.face.state.clone();
    let _tables = zwrite!(face.face.tables.tables);
    let ids = &mut get_mut_unchecked(&mut state).local_expr_ids;
    while ids.allocate(|_| false).is_some() {}
}

fn local_mappings(face: &MockFace) -> usize {
    let _tables = zread!(face.face.tables.tables);
    face.face.state.local_mappings.len()
}

/// Key expressions of the `UndeclareToken`s received by `face`, resolved against the mappings
/// live when each one arrived, or `None` when its scope was not mapped.
fn undeclared_token_keys(face: &MockFace) -> Vec<Option<String>> {
    face.recorder().with_messages(|msgs| {
        let mut live: HashMap<ExprId, String> = HashMap::new();
        let mut keys = Vec::new();
        for msg in msgs {
            let Message::Declare(Declare { body, .. }) = msg else {
                continue;
            };
            match body {
                DeclareBody::DeclareKeyExpr(d) => {
                    live.insert(d.id, d.wire_expr.suffix.to_string());
                }
                DeclareBody::UndeclareKeyExpr(u) => {
                    live.remove(&u.id);
                }
                DeclareBody::UndeclareToken(u) => {
                    let wire_expr = &u.ext_wire_expr.wire_expr;
                    keys.push(if wire_expr.scope == EMPTY_EXPR_ID {
                        Some(wire_expr.suffix.to_string())
                    } else {
                        live.get(&wire_expr.scope)
                            .map(|prefix| format!("{prefix}{}", wire_expr.suffix))
                    });
                }
                _ => {}
            }
        }
        keys
    })
}

/// Declare then undeclare `cycles` liveliness tokens, each on a key never used before.
fn churn_tokens(session: &MockFace, cycles: u32) {
    for i in 0..cycles {
        session.declare_token(None, i, format!("churn/k{i:09}/leaf"));
        session.undeclare_token(i);
    }
}

#[test]
fn should_not_keep_expr_ids_of_undeclared_tokens() {
    let (_peer, router, session) = peer_with_router_face();

    churn_tokens(&session, 1_000);

    assert_eq!(local_mappings(&router), 0);
}

#[test]
fn should_send_undeclare_keyexpr_after_the_mapped_token_is_undeclared() {
    let (_peer, router, session) = peer_with_router_face();

    churn_tokens(&session, 1);

    let mappings = router.recorder().keyexpr_mappings();
    let id = mappings[0].0;
    assert_eq!(
        mappings,
        vec![(id, Some("churn/k000000000/leaf".into())), (id, None)]
    );
}

#[test]
fn should_not_reuse_a_released_expr_id_for_the_next_declaration() {
    let (_peer, router, session) = peer_with_router_face();

    churn_tokens(&session, 2);

    let declared: Vec<ExprId> = router
        .recorder()
        .keyexpr_mappings()
        .into_iter()
        .filter_map(|(id, expr)| expr.map(|_| id))
        .collect();
    assert_ne!(declared[0], declared[1]);
}

#[test]
fn should_keep_routing_declarations_after_more_than_u16_max_churn_cycles() {
    let (_peer, router, session) = peer_with_router_face();

    churn_tokens(&session, u16::MAX as u32 + 1_000);

    assert_eq!(router.recorder().tokens().len(), u16::MAX as usize + 1_000);
}

#[test]
fn should_undeclare_an_expr_id_before_declaring_it_again() {
    let (_peer, router, session) = peer_with_router_face();

    churn_tokens(&session, u16::MAX as u32 + 1_000);

    let mut live: HashMap<ExprId, bool> = HashMap::new();
    for (id, expr) in router.recorder().keyexpr_mappings() {
        let was_live = live.insert(id, expr.is_some()).unwrap_or(false);
        assert!(
            !(was_live && expr.is_some()),
            "expr id {id} redeclared while still mapped"
        );
    }
}

#[test]
fn should_fall_back_to_the_full_key_when_every_expr_id_is_in_use() {
    let (_peer, router, session) = peer_with_router_face();
    exhaust_expr_ids(&router);

    session.declare_token(None, 0, "live/overflow/leaf");

    let token = router.recorder().tokens().pop().unwrap();
    assert_eq!(
        (token.wire_expr.scope, token.wire_expr.suffix.to_string()),
        (EMPTY_EXPR_ID, "live/overflow/leaf".to_string())
    );
}

#[test]
fn should_map_new_keys_again_once_an_expr_id_is_released() {
    let (_peer, router, session) = peer_with_router_face();
    session.declare_token(None, 0, "live/first/leaf");
    exhaust_expr_ids(&router);
    session.undeclare_token(0);

    session.declare_token(None, 1, "live/second/leaf");

    let token = router.recorder().tokens().pop().unwrap();
    assert_ne!(token.wire_expr.scope, EMPTY_EXPR_ID);
}

#[test]
fn should_not_send_undeclare_keyexpr_to_a_closing_face() {
    let (_peer, router, session) = peer_with_router_face();
    session.declare_token(None, 0, "a/b/c");
    session.declare_token(None, 1, "a/b");
    router.declare_subscriber(None, 0, "a/b/c");
    session.undeclare_token(0);
    session.undeclare_token(1);
    router.recorder().clear();

    router.face.send_close();

    assert!(router
        .recorder()
        .keyexpr_mappings()
        .iter()
        .all(|(_, expr)| expr.is_some()));
}

#[test]
fn should_resolve_the_key_of_a_token_undeclared_to_a_face_that_never_saw_it_declared() {
    try_init_tracing_subscriber();
    let router = Harness::new_router();
    let client = router.new_face(
        FaceDef::default()
            .mode(WhatAmI::Client)
            .region(Region::default_south(WhatAmI::Client)),
    );
    let session = router.new_session();
    session.declare_token(None, 0, "a/b");
    let options = InterestOptions::KEYEXPRS + InterestOptions::TOKENS;
    client.interest(1, InterestMode::Future, options, "a/**");
    client.interest(2, InterestMode::Current, options, "a/**");

    session.undeclare_token(0);

    assert_eq!(
        undeclared_token_keys(&client),
        vec![Some("a/b".to_string())]
    );
}
