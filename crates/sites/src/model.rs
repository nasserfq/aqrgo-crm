use serde::{Deserialize, Serialize};

/// The brand lavender, used until the owner picks their own colour.
pub const DEFAULT_ACCENT: &str = "#6c4fd8";

/// Everything the owner fills in on the site form.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteInput {
    pub slug: String,
    pub published: bool,
    pub company_name: String,
    pub tagline: String,
    pub about: String,
    pub services: String,
    pub license_no: String,
    pub phone: String,
    pub whatsapp: String,
    pub email: String,
    pub address: String,
    pub map_url: String,
    pub accent: String,
    /// Listing ids to keep off the public page.
    pub hidden_listings: Vec<i64>,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Site {
    pub owner_id: i64,
    pub slug: String,
    pub published: bool,
    pub company_name: String,
    pub tagline: String,
    pub about: String,
    pub services: String,
    pub license_no: String,
    pub phone: String,
    pub whatsapp: String,
    pub email: String,
    pub address: String,
    pub map_url: String,
    pub accent: String,
    pub has_logo: bool,
}

impl Site {
    /// Non-empty service lines.
    pub fn services(&self) -> Vec<&str> {
        self.services
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn whatsapp_link(&self) -> Option<String> {
        international(&self.whatsapp).map(|n| format!("https://wa.me/{n}"))
    }

    pub fn phone_link(&self) -> Option<String> {
        international(&self.phone).map(|n| format!("tel:+{n}"))
    }

    pub fn to_input(&self, hidden_listings: Vec<i64>) -> SiteInput {
        SiteInput {
            slug: self.slug.clone(),
            published: self.published,
            company_name: self.company_name.clone(),
            tagline: self.tagline.clone(),
            about: self.about.clone(),
            services: self.services.clone(),
            license_no: self.license_no.clone(),
            phone: self.phone.clone(),
            whatsapp: self.whatsapp.clone(),
            email: self.email.clone(),
            address: self.address.clone(),
            map_url: self.map_url.clone(),
            accent: self.accent.clone(),
            hidden_listings,
        }
    }
}

/// A phone number in international form without "+", assuming Saudi
/// numbers when no country code is given (05xxxxxxxx → 9665xxxxxxxx).
fn international(raw: &str) -> Option<String> {
    let d: String = common::digits::to_ascii_digits(raw)
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    let d = d.strip_prefix("00").map(str::to_string).unwrap_or(d);
    let n = if d.len() == 10 && d.starts_with("05") {
        format!("966{}", &d[1..])
    } else if d.len() == 9 && d.starts_with('5') {
        format!("966{d}")
    } else {
        d
    };
    (n.len() >= 8).then_some(n)
}

/// What the public may see of one property: never buyer, seller, broker,
/// plot number, income or private notes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicListing {
    pub offer_type: String,
    pub location: String,
    pub area: String,
    pub price: String,
    pub rooms: String,
    pub bathrooms: String,
    pub floors: String,
    pub frontages: String,
}

impl From<&listings::Listing> for PublicListing {
    fn from(l: &listings::Listing) -> Self {
        Self {
            offer_type: l.offer_type.clone(),
            location: l.location.clone(),
            area: l.area.clone(),
            price: l.price.clone(),
            rooms: l.rooms.clone(),
            bathrooms: l.bathrooms.clone(),
            floors: l.floors.clone(),
            frontages: l.frontages.clone(),
        }
    }
}

/// A published site with the listings it shows.
#[derive(Clone, Debug)]
pub struct PublicSite {
    pub site: Site,
    pub listings: Vec<PublicListing>,
}

/// Addresses kept for the platform itself.
const RESERVED_SLUGS: &[&str] = &[
    "www",
    "api",
    "app",
    "admin",
    "mail",
    "email",
    "smtp",
    "ftp",
    "crm",
    "auth",
    "login",
    "static",
    "assets",
    "cdn",
    "help",
    "support",
    "blog",
    "status",
    "dev",
    "staging",
    "test",
    "aqrgo",
    "dashboard",
    "account",
    "site",
    "sites",
    "s",
];

/// Checks an address: 3 to 30 of a-z, 0-9 and "-", not at either end.
pub fn clean_slug(raw: &str) -> Result<String, &'static str> {
    let s = raw.trim().to_lowercase();
    let ok_chars = s
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !(3..=30).contains(&s.len()) || !ok_chars || s.starts_with('-') || s.ends_with('-') {
        return Err("العنوان من ٣ إلى ٣٠ حرفاً إنجليزياً صغيراً أو أرقام أو شرطة (-)");
    }
    if RESERVED_SLUGS.contains(&s.as_str()) {
        return Err("هذا العنوان محجوز، اختر عنواناً آخر");
    }
    Ok(s)
}

fn clean_accent(raw: &str) -> String {
    let a = raw.trim().to_lowercase();
    let hex = a.strip_prefix('#').unwrap_or("");
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        a
    } else {
        DEFAULT_ACCENT.to_string()
    }
}

impl SiteInput {
    /// Trims every field, checks the address, and drops values that are not
    /// safe to put into the page (colour, map link).
    pub fn normalized(mut self) -> Result<Self, &'static str> {
        self.slug = clean_slug(&self.slug)?;
        for f in [
            &mut self.company_name,
            &mut self.tagline,
            &mut self.about,
            &mut self.services,
            &mut self.license_no,
            &mut self.phone,
            &mut self.whatsapp,
            &mut self.email,
            &mut self.address,
            &mut self.map_url,
        ] {
            *f = f.trim().to_string();
        }
        self.license_no = common::digits::to_ascii_digits(&self.license_no);
        self.accent = clean_accent(&self.accent);
        if !self.map_url.starts_with("https://") {
            self.map_url.clear();
        }
        if self.published && self.company_name.is_empty() {
            return Err("اكتب اسم المكتب قبل نشر الموقع");
        }
        Ok(self)
    }
}

/// The picture types a logo may be, recognised by their first bytes.
/// SVG is refused on purpose: it can carry scripts.
pub fn logo_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Largest logo accepted, in bytes.
pub const MAX_LOGO_BYTES: usize = 512 * 1024;

/// The site address in `host` if it is one label under `base_domain`
/// (`alnukhba.aqrgo.com` → `alnukhba`). Says nothing about whether it exists.
pub fn slug_in_host(host: &str, base_domain: &str) -> Option<String> {
    let host = host.trim().to_lowercase();
    let host = host.split(':').next().unwrap_or("");
    let label = host.strip_suffix(&format!(".{}", base_domain.to_lowercase()))?;
    if label.contains('.') {
        return None;
    }
    clean_slug(label).ok()
}
