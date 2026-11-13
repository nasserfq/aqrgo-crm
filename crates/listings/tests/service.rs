use listings::{Filter, ListingInput, ListingsService, Source, Status};

async fn service() -> ListingsService {
    let pool = testkit::fresh_pool().await;
    common::db::migrate_module(&pool, listings::SCHEMA, &listings::MIGRATOR)
        .await
        .unwrap();
    ListingsService::new(pool)
}

fn land(location: &str) -> ListingInput {
    ListingInput {
        offer_type: "أرض للبيع".into(),
        location: location.into(),
        plan_no: "٢٤٥٦".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn each_owner_numbers_their_rows_from_one() {
    let svc = service().await;
    let a1 = svc.create(1, land("الملقا"), Source::Web).await.unwrap();
    let b1 = svc
        .create(2, land("النرجس"), Source::Telegram)
        .await
        .unwrap();
    let a2 = svc.create(1, land("حطين"), Source::Web).await.unwrap();
    assert_eq!((a1.ref_no, a2.ref_no, b1.ref_no), (1, 2, 1));
    assert_eq!(a2.number(), "٢");
}

#[tokio::test]
async fn owners_cannot_see_or_touch_each_others_rows() {
    let svc = service().await;
    let mine = svc.create(1, land("الملقا"), Source::Web).await.unwrap();

    assert!(svc.list(2, &Filter::default()).await.unwrap().is_empty());
    assert!(svc.get(2, mine.id).await.unwrap().is_none());
    assert!(svc
        .update(2, mine.id, land("مسروق"))
        .await
        .unwrap()
        .is_none());
    assert!(!svc.delete(2, mine.id).await.unwrap());

    let still = svc.get(1, mine.id).await.unwrap().unwrap();
    assert_eq!(still.location, "الملقا");
}

#[tokio::test]
async fn search_matches_any_column_and_arabic_digits() {
    let svc = service().await;
    svc.create(1, land("الملقا"), Source::Web).await.unwrap();
    svc.create(
        1,
        ListingInput {
            seller: "أبو ناصر 0555".into(),
            ..land("النرجس")
        },
        Source::Web,
    )
    .await
    .unwrap();

    let by_name = svc
        .list(
            1,
            &Filter {
                q: Some("ناصر".into()),
                status: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].location, "النرجس");

    // Digits are matched whether typed as Arabic-Indic or ASCII, either way round.
    let by_plan = svc
        .list(
            1,
            &Filter {
                q: Some("2456".into()),
                status: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(by_plan.len(), 2);
    let by_phone = svc
        .list(
            1,
            &Filter {
                q: Some("٠٥٥٥".into()),
                status: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(by_phone.len(), 1);
}

#[tokio::test]
async fn edit_changes_status_and_filter_by_status() {
    let svc = service().await;
    let l = svc.create(1, land("الملقا"), Source::Web).await.unwrap();
    assert_eq!(l.status(), Status::Available);

    let sold = svc
        .update(
            1,
            l.id,
            ListingInput {
                status: Status::Sold,
                buyer: "محمد".into(),
                ..land("الملقا")
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sold.status(), Status::Sold);
    assert_eq!(sold.ref_no, l.ref_no);

    let available = svc
        .list(
            1,
            &Filter {
                q: None,
                status: Some(Status::Available),
            },
        )
        .await
        .unwrap();
    assert!(available.is_empty());
}

#[tokio::test]
async fn rejects_an_empty_row() {
    let svc = service().await;
    assert!(svc
        .create(
            1,
            ListingInput {
                price: "  ".into(),
                ..Default::default()
            },
            Source::Web
        )
        .await
        .is_err());
}

#[tokio::test]
async fn simultaneous_saves_all_get_their_own_number() {
    let svc = service().await;
    let mut handles = Vec::new();
    for i in 0..12 {
        let svc = svc.clone();
        handles.push(tokio::spawn(async move {
            svc.create(1, land(&format!("حي {i}")), Source::Web).await
        }));
    }
    let mut numbers = Vec::new();
    for h in handles {
        numbers.push(h.await.unwrap().expect("every save succeeds").ref_no);
    }
    numbers.sort();
    assert_eq!(numbers, (1..=12).collect::<Vec<_>>());
}

#[tokio::test]
async fn keeps_building_details_and_finds_them() {
    let svc = service().await;
    let input = ListingInput {
        offer_type: "عمارة للبيع".into(),
        location: "حي الخليج - شرق الرياض".into(),
        rooms: "٤٤".into(),
        bathrooms: "33".into(),
        floors: "دورين ونص".into(),
        units: "7 شقق".into(),
        income: "عقد واحد 5 سنوات بقيمة 600 ألف دفعتين".into(),
        details: "مصعد راكب\nتشطيب حديث\nمباشر مع الوكيل".into(),
        ..Default::default()
    };
    let saved = svc.create(1, input, Source::Telegram).await.unwrap();
    let got = svc.get(1, saved.id).await.unwrap().unwrap().to_input();
    assert_eq!(got.rooms, "44", "digits normalised like other columns");
    assert_eq!(got.bathrooms, "33");
    assert_eq!(got.floors, "دورين ونص");
    assert_eq!(got.units, "7 شقق");
    assert!(got.income.contains("600"));
    assert!(got.details.contains("مصعد"));

    let filter = Filter {
        q: Some("مصعد".into()),
        ..Default::default()
    };
    assert_eq!(svc.list(1, &filter).await.unwrap().len(), 1);
}
