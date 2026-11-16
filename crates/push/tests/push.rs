//! Push notifications: phones register, new leads from Telegram/WhatsApp
//! notify the owner's phones through a background job, dead tokens are dropped.

use std::sync::{Arc, Mutex};

use listings::{ListingInput, ListingsService, Source};
use push::{Message, PushSender, PushService, SendOutcome};

#[derive(Default, Clone)]
struct Outbox {
    sent: Arc<Mutex<Vec<Message>>>,
    dead: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl PushSender for Outbox {
    async fn send(&self, messages: &[Message]) -> Result<Vec<SendOutcome>, String> {
        self.sent.lock().unwrap().extend_from_slice(messages);
        let dead = self.dead.lock().unwrap().clone();
        Ok(messages
            .iter()
            .map(|m| {
                if dead.contains(&m.to) {
                    SendOutcome::DeviceGone
                } else {
                    SendOutcome::Ok
                }
            })
            .collect())
    }
}

struct World {
    push: PushService,
    listings: ListingsService,
    worker: jobs::Worker,
    outbox: Outbox,
}

async fn world() -> World {
    let pool = testkit::fresh_pool().await;
    for (schema, m) in [
        (jobs::SCHEMA, &jobs::MIGRATOR),
        (listings::SCHEMA, &listings::MIGRATOR),
        (push::SCHEMA, &push::MIGRATOR),
    ] {
        common::db::migrate_module(&pool, schema, m).await.unwrap();
    }
    let jobs = jobs::Jobs::new(pool.clone());
    let push = PushService::new(pool.clone(), jobs.clone());
    let listings = ListingsService::new(pool.clone()).with_listener(Arc::new(push.clone()));
    let outbox = Outbox::default();
    let worker = push::register(jobs.worker(), push.clone(), Arc::new(outbox.clone()));
    World {
        push,
        listings,
        worker,
        outbox,
    }
}

fn lead(offer: &str, location: &str) -> ListingInput {
    ListingInput {
        offer_type: offer.into(),
        location: location.into(),
        ..Default::default()
    }
}

const PHONE: &str = "ExponentPushToken[aaaa]";
const TABLET: &str = "ExponentPushToken[bbbb]";

#[tokio::test]
async fn a_lead_from_telegram_notifies_every_phone_of_its_owner() {
    let w = world().await;
    w.push.register_device(1, PHONE, "ios").await.unwrap();
    w.push.register_device(1, TABLET, "android").await.unwrap();
    w.push
        .register_device(2, "ExponentPushToken[other]", "ios")
        .await
        .unwrap();

    let row = w
        .listings
        .create(1, lead("أرض للبيع", "حي الملقا"), Source::Telegram)
        .await
        .unwrap();
    while w.worker.run_one().await.unwrap() {}

    let sent = w.outbox.sent.lock().unwrap().clone();
    let mut to: Vec<_> = sent.iter().map(|m| m.to.as_str()).collect();
    to.sort();
    assert_eq!(to, vec![PHONE, TABLET], "only the owner's phones");
    assert_eq!(sent[0].title, "عرض جديد من تيليجرام");
    assert_eq!(sent[0].body, "أرض للبيع · حي الملقا");
    assert_eq!(sent[0].data["listing_id"], row.id);
}

#[tokio::test]
async fn rows_added_by_hand_do_not_notify() {
    let w = world().await;
    w.push.register_device(1, PHONE, "ios").await.unwrap();
    w.listings
        .create(1, lead("أرض للبيع", "حي الملقا"), Source::Web)
        .await
        .unwrap();
    while w.worker.run_one().await.unwrap() {}
    assert!(w.outbox.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_phones_means_nothing_to_send() {
    let w = world().await;
    w.listings
        .create(1, lead("أرض للبيع", ""), Source::Telegram)
        .await
        .unwrap();
    while w.worker.run_one().await.unwrap() {}
    assert!(w.outbox.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_phone_follows_whoever_signed_in_last_and_can_opt_out() {
    let w = world().await;
    w.push.register_device(1, PHONE, "ios").await.unwrap();
    // Same phone, another account signs in.
    w.push.register_device(2, PHONE, "ios").await.unwrap();
    assert!(w.push.devices(1).await.unwrap().is_empty());
    assert_eq!(w.push.devices(2).await.unwrap(), vec![PHONE.to_string()]);

    // Someone else can't remove your phone; you can.
    w.push.unregister_device(1, PHONE).await.unwrap();
    assert_eq!(w.push.devices(2).await.unwrap().len(), 1);
    w.push.unregister_device(2, PHONE).await.unwrap();
    assert!(w.push.devices(2).await.unwrap().is_empty());

    assert!(w
        .push
        .register_device(1, "not-a-token", "ios")
        .await
        .is_err());
    assert!(w.push.register_device(1, PHONE, "windows").await.is_err());
}

#[tokio::test]
async fn phones_that_uninstalled_the_app_are_forgotten() {
    let w = world().await;
    w.push.register_device(1, PHONE, "ios").await.unwrap();
    w.push.register_device(1, TABLET, "android").await.unwrap();
    w.outbox.dead.lock().unwrap().push(TABLET.into());

    w.listings
        .create(1, lead("فيلا للبيع", "حي حطين"), Source::Telegram)
        .await
        .unwrap();
    while w.worker.run_one().await.unwrap() {}

    assert_eq!(w.push.devices(1).await.unwrap(), vec![PHONE.to_string()]);
}
