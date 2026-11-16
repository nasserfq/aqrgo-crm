use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Quick choices for نوع العرض. Free text is always allowed too.
pub const OFFER_TYPE_SUGGESTIONS: &[&str] = &[
    "أرض للبيع",
    "فيلا للبيع",
    "شقة للبيع",
    "عمارة للبيع",
    "دبلكس للبيع",
    "أرض للإيجار",
    "فيلا للإيجار",
    "شقة للإيجار",
    "محل للإيجار",
    "طلب شراء",
];

/// The free-text columns, in the order of the original Word table, then the
/// columns added later. (key, Arabic label)
pub const FIELDS: &[(&str, &str)] = &[
    ("offer_type", "نوع العرض"),
    ("plan_no", "رقم المخطط"),
    ("plot_no", "رقم القطعة"),
    ("area", "المساحة"),
    ("frontages", "الواجهات والشوارع"),
    ("location", "الموقع"),
    ("price", "السعر"),
    ("buyer", "بيانات المشتري"),
    ("seller", "بيانات البائع"),
    ("broker", "بيانات الوسيط"),
    ("rooms", "عدد الغرف"),
    ("bathrooms", "دورات المياه"),
    ("floors", "الأدوار"),
    ("units", "الوحدات"),
    ("income", "الدخل والعقود"),
    ("details", "تفاصيل إضافية"),
];

/// Building details the step-by-step Telegram flow doesn't ask one by one
/// (they can still be filled from a message or edited).
pub const BUILDING_FIELDS: &[&str] = &["rooms", "bathrooms", "floors", "units", "income"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Available,
    Reserved,
    Sold,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Available, Status::Reserved, Status::Sold];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Available => "available",
            Status::Reserved => "reserved",
            Status::Sold => "sold",
        }
    }

    pub fn label_ar(self) -> &'static str {
        match self {
            Status::Available => "متاح",
            Status::Reserved => "محجوز",
            Status::Sold => "مباع",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Status::ALL
            .into_iter()
            .find(|st| st.as_str() == s || st.label_ar() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Web,
    Telegram,
    WhatsApp,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Web => "web",
            Source::Telegram => "telegram",
            Source::WhatsApp => "whatsapp",
        }
    }
}

/// Everything a person can enter or edit for one property.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingInput {
    pub offer_type: String,
    pub plan_no: String,
    pub plot_no: String,
    pub area: String,
    pub frontages: String,
    pub location: String,
    pub price: String,
    pub buyer: String,
    pub seller: String,
    pub broker: String,
    #[serde(default)]
    pub rooms: String,
    #[serde(default)]
    pub bathrooms: String,
    #[serde(default)]
    pub floors: String,
    #[serde(default)]
    pub units: String,
    #[serde(default)]
    pub income: String,
    #[serde(default)]
    pub details: String,
    pub listed_on: Option<NaiveDate>,
    #[serde(default)]
    pub status: Status,
}

impl ListingInput {
    pub fn get(&self, key: &str) -> &str {
        match key {
            "offer_type" => &self.offer_type,
            "plan_no" => &self.plan_no,
            "plot_no" => &self.plot_no,
            "area" => &self.area,
            "frontages" => &self.frontages,
            "location" => &self.location,
            "price" => &self.price,
            "buyer" => &self.buyer,
            "seller" => &self.seller,
            "broker" => &self.broker,
            "rooms" => &self.rooms,
            "bathrooms" => &self.bathrooms,
            "floors" => &self.floors,
            "units" => &self.units,
            "income" => &self.income,
            "details" => &self.details,
            _ => "",
        }
    }

    pub fn set(&mut self, key: &str, value: String) {
        let slot = match key {
            "offer_type" => &mut self.offer_type,
            "plan_no" => &mut self.plan_no,
            "plot_no" => &mut self.plot_no,
            "area" => &mut self.area,
            "frontages" => &mut self.frontages,
            "location" => &mut self.location,
            "price" => &mut self.price,
            "buyer" => &mut self.buyer,
            "seller" => &mut self.seller,
            "broker" => &mut self.broker,
            "rooms" => &mut self.rooms,
            "bathrooms" => &mut self.bathrooms,
            "floors" => &mut self.floors,
            "units" => &mut self.units,
            "income" => &mut self.income,
            "details" => &mut self.details,
            _ => return,
        };
        *slot = value;
    }

    /// Trims every field and stores digits as ASCII so search works either way.
    /// A row must say at least what is offered, where, or which plan.
    pub fn normalized(mut self) -> Result<Self, &'static str> {
        for (key, _) in FIELDS {
            let v = common::digits::to_ascii_digits(self.get(key).trim());
            self.set(key, v);
        }
        if self.offer_type.is_empty() && self.location.is_empty() && self.plan_no.is_empty() {
            return Err("اكتب نوع العرض أو الموقع أو رقم المخطط على الأقل");
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct Listing {
    pub id: i64,
    pub ref_no: i32,
    pub offer_type: String,
    pub plan_no: String,
    pub plot_no: String,
    pub area: String,
    pub frontages: String,
    pub location: String,
    pub price: String,
    pub buyer: String,
    pub seller: String,
    pub broker: String,
    pub rooms: String,
    pub bathrooms: String,
    pub floors: String,
    pub units: String,
    pub income: String,
    pub details: String,
    pub listed_on: NaiveDate,
    pub status: String,
    pub source: String,
}

impl Listing {
    pub fn status(&self) -> Status {
        Status::parse(&self.status).unwrap_or_default()
    }

    /// Plain label for exports.
    pub fn date_label(&self) -> String {
        common::dates::full_ar(self.listed_on)
    }

    pub fn date_hijri(&self) -> String {
        format!(
            "{} {}",
            common::dates::weekday_ar(self.listed_on),
            common::dates::hijri_ar(self.listed_on)
        )
    }

    pub fn date_gregorian(&self) -> String {
        common::dates::gregorian(self.listed_on)
    }

    pub fn number(&self) -> String {
        common::digits::to_arabic_indic(&self.ref_no.to_string())
    }

    pub fn to_input(&self) -> ListingInput {
        ListingInput {
            offer_type: self.offer_type.clone(),
            plan_no: self.plan_no.clone(),
            plot_no: self.plot_no.clone(),
            area: self.area.clone(),
            frontages: self.frontages.clone(),
            location: self.location.clone(),
            price: self.price.clone(),
            buyer: self.buyer.clone(),
            seller: self.seller.clone(),
            broker: self.broker.clone(),
            rooms: self.rooms.clone(),
            bathrooms: self.bathrooms.clone(),
            floors: self.floors.clone(),
            units: self.units.clone(),
            income: self.income.clone(),
            details: self.details.clone(),
            listed_on: Some(self.listed_on),
            status: self.status(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_something_identifying() {
        assert!(ListingInput::default().normalized().is_err());
        let ok = ListingInput {
            location: "  حي الملقا ".into(),
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(ok.location, "حي الملقا");
    }

    #[test]
    fn status_parses_arabic_and_code() {
        assert_eq!(Status::parse("مباع"), Some(Status::Sold));
        assert_eq!(Status::parse("reserved"), Some(Status::Reserved));
        assert_eq!(Status::parse("x"), None);
    }
}
