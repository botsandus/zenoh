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

#![cfg(feature = "unstable")]

use std::time::Duration;
use zenoh_ext::{
    AdvancedPublisherBuilderExt, AdvancedSubscriberBuilderExt, CacheConfig, HistoryConfig,
    MissDetectionConfig,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_retry_preserves_max_age() {
    let mut config = zenoh::Config::default();
    config.scouting.multicast.set_enabled(Some(false)).unwrap();
    config.scouting.gossip.set_enabled(Some(false)).unwrap();
    config.listen.endpoints.set(vec![]).unwrap();
    config.insert_json5("timestamping/enabled", "true").unwrap();
    let session = zenoh::open(config).await.unwrap();
    let publisher = session
        .declare_publisher("review/history/bounds")
        .cache(CacheConfig::default().max_samples(10))
        .sample_miss_detection(MissDetectionConfig::default())
        .publisher_detection()
        .await
        .unwrap();
    publisher.put("stale cached sample").await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let subscriber = session
        .declare_subscriber("review/history/bounds")
        .history(
            HistoryConfig::default()
                .detect_late_publishers()
                .max_age(0.1),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        subscriber.try_recv().unwrap().is_none(),
        "initial age-filtered query unexpectedly returned stale data"
    );
    let stale = tokio::time::timeout(Duration::from_secs(5), subscriber.recv_async()).await;
    assert!(
        stale.is_err(),
        "history retry delivered a sample excluded by max_age"
    );
    publisher.put("fresh sample").await.unwrap();
    let fresh = tokio::time::timeout(Duration::from_secs(5), subscriber.recv_async())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fresh.payload().try_to_string().unwrap(), "fresh sample");
    drop(subscriber);
    drop(publisher);
    session.close().await.unwrap();
}
