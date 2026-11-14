use listings::{ListingInput, ListingsService, Source, Status};
use sites::{SiteInput, SitesError, SitesService};

async fn services() -> (SitesService, ListingsService) {
    let pool = testkit::fresh_pool().await;
    for (schema, m) in [
        (listings::SCHEMA, &listings::MIGRATOR),
        (sites::SCHEMA, &sites::MIGRATOR),
    ] {
        common::db::migrate_module(&pool, schema, m).await.unwrap();
    }
    let listings = ListingsService::new(pool.clone());
    (SitesService::new(pool, listings.clone()), listings)
}

fn office(slug: &str) -> SiteInput {
    SiteInput {
        slug: slug.into(),
        published: true,
        company_name: "مكتب النخبة العقاري".into(),
        license_no: "1200012345".into(),
        services: "تسويق العقارات\nإدارة الأملاك\n\n".into(),
        whatsapp: "0551234567".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_published_site_is_found_by_its_address() {
    let (svc, _) = services().await;
    svc.save(1, office("alnukhba")).await.unwrap();

    let site = svc.public("alnukhba").await.unwrap().expect("published");
    assert_eq!(site.site.company_name, "مكتب النخبة العقاري");
    assert_eq!(site.site.services(), vec!["تسويق العقارات", "إدارة الأملاك"]);
    assert_eq!(
        site.site.whatsapp_link().as_deref(),
        Some("https://wa.me/966551234567")
    );
}

#[tokio::test]
async fn an_unpublished_site_is_not_public() {
    let (svc, _) = services().await;
    svc.save(
        1,
        SiteInput {
            published: false,
            ..office("draft-office")
        },
    )
    .await
    .unwrap();
    assert!(svc.public("draft-office").await.unwrap().is_none());
    assert!(svc.public("nobody").await.unwrap().is_none());
}

fn villa(location: &str, status: Status) -> ListingInput {
    ListingInput {
        offer_type: "فيلا للبيع".into(),
        location: location.into(),
        price: "1500000".into(),
        rooms: "5".into(),
        plot_no: "77".into(),
        seller: "أبو فهد 0500000001".into(),
        buyer: "سالم 0500000002".into(),
        broker: "وسيط 0500000003".into(),
        details: "المالك مستعجل".into(),
        status,
        ..Default::default()
    }
}

#[tokio::test]
async fn the_site_shows_only_the_owners_available_listings_that_are_not_hidden() {
    let (svc, listings) = services().await;
    let shown = listings
        .create(1, villa("الملقا", Status::Available), Source::Web)
        .await
        .unwrap();
    let hidden = listings
        .create(1, villa("حطين", Status::Available), Source::Web)
        .await
        .unwrap();
    listings
        .create(1, villa("النرجس", Status::Sold), Source::Web)
        .await
        .unwrap();
    listings
        .create(2, villa("العارض", Status::Available), Source::Web)
        .await
        .unwrap();
    svc.save(
        1,
        SiteInput {
            hidden_listings: vec![hidden.id],
            ..office("alnukhba")
        },
    )
    .await
    .unwrap();

    let site = svc.public("alnukhba").await.unwrap().unwrap();
    let locations: Vec<_> = site.listings.iter().map(|l| l.location.as_str()).collect();
    assert_eq!(locations, vec!["الملقا"]);
    assert_eq!(svc.hidden_listings(1).await.unwrap(), vec![hidden.id]);
    let _ = shown;
}

#[tokio::test]
async fn the_public_never_sees_private_columns() {
    let (svc, listings) = services().await;
    listings
        .create(1, villa("الملقا", Status::Available), Source::Web)
        .await
        .unwrap();
    svc.save(1, office("alnukhba")).await.unwrap();

    let site = svc.public("alnukhba").await.unwrap().unwrap();
    let everything = format!("{:?}", site.listings);
    assert!(everything.contains("الملقا") && everything.contains("1500000"));
    for private in ["0500000001", "0500000002", "0500000003", "مستعجل", "77"] {
        assert!(!everything.contains(private), "leaked {private}");
    }
}

#[tokio::test]
async fn the_address_is_cleaned_and_checked() {
    let (svc, _) = services().await;
    let site = svc.save(1, office("  AlNukhba-RE ")).await.unwrap();
    assert_eq!(site.slug, "alnukhba-re");

    for bad in [
        "ab",
        "www",
        "admin",
        "-office",
        "office-",
        "مكتب",
        "my office",
        &"a".repeat(31),
    ] {
        let err = svc.save(1, office(bad)).await.unwrap_err();
        assert!(matches!(err, SitesError::Invalid(_)), "{bad:?} accepted");
    }
}

#[tokio::test]
async fn two_owners_cannot_share_an_address() {
    let (svc, _) = services().await;
    svc.save(1, office("alnukhba")).await.unwrap();
    let err = svc.save(2, office("alnukhba")).await.unwrap_err();
    assert!(matches!(err, SitesError::Invalid(_)));
    // The first owner can still save their own site under it.
    svc.save(1, office("alnukhba")).await.unwrap();
}

#[tokio::test]
async fn a_published_site_needs_a_name_and_unsafe_values_are_dropped() {
    let (svc, _) = services().await;
    let err = svc
        .save(
            1,
            SiteInput {
                company_name: "  ".into(),
                ..office("alnukhba")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, SitesError::Invalid(_)));

    let site = svc
        .save(
            1,
            SiteInput {
                accent: "red;}</style><script>".into(),
                map_url: "javascript:alert(1)".into(),
                ..office("alnukhba")
            },
        )
        .await
        .unwrap();
    assert_eq!(site.accent, sites::DEFAULT_ACCENT);
    assert_eq!(site.map_url, "");

    let site = svc
        .save(
            1,
            SiteInput {
                accent: "#0F766E".into(),
                map_url: "https://maps.app.goo.gl/abc".into(),
                ..office("alnukhba")
            },
        )
        .await
        .unwrap();
    assert_eq!(site.accent, "#0f766e");
    assert_eq!(site.map_url, "https://maps.app.goo.gl/abc");
}

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

#[tokio::test]
async fn the_logo_must_be_a_small_picture() {
    let (svc, _) = services().await;
    svc.save(1, office("alnukhba")).await.unwrap();
    assert!(svc.logo("alnukhba").await.unwrap().is_none());

    let svg = b"<svg onload=alert(1)></svg>".to_vec();
    assert!(matches!(
        svc.set_logo(1, svg).await,
        Err(SitesError::Invalid(_))
    ));
    let huge = [PNG, &vec![0u8; 600 * 1024]].concat();
    assert!(matches!(
        svc.set_logo(1, huge).await,
        Err(SitesError::Invalid(_))
    ));

    svc.set_logo(1, PNG.to_vec()).await.unwrap();
    let sites::Logo(bytes, content_type) = svc.logo("alnukhba").await.unwrap().unwrap();
    assert_eq!(
        (bytes.as_slice(), content_type.as_str()),
        (PNG, "image/png")
    );
    assert!(svc.public("alnukhba").await.unwrap().unwrap().site.has_logo);
}

#[tokio::test]
async fn only_published_site_hosts_are_approved_for_certificates() {
    let (svc, _) = services().await;
    svc.save(1, office("alnukhba")).await.unwrap();
    svc.save(
        2,
        SiteInput {
            published: false,
            ..office("draft")
        },
    )
    .await
    .unwrap();

    assert_eq!(
        svc.slug_for_host("alnukhba.aqrgo.com", "aqrgo.com")
            .await
            .unwrap()
            .as_deref(),
        Some("alnukhba")
    );
    assert_eq!(
        svc.slug_for_host("ALNUKHBA.aqrgo.com:443", "aqrgo.com")
            .await
            .unwrap()
            .as_deref(),
        Some("alnukhba")
    );
    for other in [
        "draft.aqrgo.com",
        "nobody.aqrgo.com",
        "aqrgo.com",
        "www.aqrgo.com",
        "alnukhba.evil.com",
        "x.alnukhba.aqrgo.com",
    ] {
        assert!(
            svc.slug_for_host(other, "aqrgo.com")
                .await
                .unwrap()
                .is_none(),
            "{other}"
        );
    }
}

#[tokio::test]
async fn an_email_that_could_rewrite_the_mail_link_is_dropped() {
    let (svc, _) = services().await;
    let site = svc
        .save(
            1,
            SiteInput {
                email: "a@b.sa?body=hi".into(),
                ..office("alnukhba")
            },
        )
        .await
        .unwrap();
    assert_eq!(site.email, "");
    let site = svc
        .save(
            1,
            SiteInput {
                email: " info@alnukhba.sa ".into(),
                ..office("alnukhba")
            },
        )
        .await
        .unwrap();
    assert_eq!(site.email, "info@alnukhba.sa");
}

#[tokio::test]
async fn the_owner_sees_their_logo_before_publishing() {
    let (svc, _) = services().await;
    svc.save(
        1,
        SiteInput {
            published: false,
            ..office("alnukhba")
        },
    )
    .await
    .unwrap();
    svc.set_logo(1, PNG.to_vec()).await.unwrap();
    assert!(
        svc.logo("alnukhba").await.unwrap().is_none(),
        "not public yet"
    );
    let sites::Logo(bytes, _) = svc.own_logo(1).await.unwrap().unwrap();
    assert_eq!(bytes, PNG);
    assert!(svc.own_logo(2).await.unwrap().is_none());
}
