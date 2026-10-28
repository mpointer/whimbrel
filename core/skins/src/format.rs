//! The skin manifest types. Ports `apps/macos/Whimbrel/Support/Skin.swift`: the same fields, the same
//! allowed values, the same limits. A `Skin` serialises to the manifest shape (`skins/<id>.json`),
//! with every font face spelled out as `Family/weight[/italic]`.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use unicode_segmentation::UnicodeSegmentation;

/// Card corner radius, in points.
pub const RADIUS_RANGE: (f64, f64) = (0.0, 24.0);
/// Border width, in points.
pub const WIDTH_RANGE: (f64, f64) = (0.0, 4.0);
/// Greetings are measured after `{name}` is filled with a name this long.
pub const MEASURED_NAME_LENGTH: usize = 30;

/// The only fonts a skin may name (the manifest spelling). Nothing is bundled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    SfPro,
    SfProRounded,
    SfMono,
    NewYork,
    Georgia,
    CourierNew,
    Menlo,
    HelveticaNeue,
    AvenirNext,
}

impl Family {
    pub const ALL: [Family; 9] = [
        Family::SfPro,
        Family::SfProRounded,
        Family::SfMono,
        Family::NewYork,
        Family::Georgia,
        Family::CourierNew,
        Family::Menlo,
        Family::HelveticaNeue,
        Family::AvenirNext,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Family::SfPro => "SF Pro",
            Family::SfProRounded => "SF Pro Rounded",
            Family::SfMono => "SF Mono",
            Family::NewYork => "New York",
            Family::Georgia => "Georgia",
            Family::CourierNew => "Courier New",
            Family::Menlo => "Menlo",
            Family::HelveticaNeue => "Helvetica Neue",
            Family::AvenirNext => "Avenir Next",
        }
    }

    pub fn parse(s: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.name() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    UltraLight,
    Thin,
    Light,
    Regular,
    Medium,
    Semibold,
    Bold,
    Heavy,
    Black,
}

impl Weight {
    pub const ALL: [Weight; 9] = [
        Weight::UltraLight,
        Weight::Thin,
        Weight::Light,
        Weight::Regular,
        Weight::Medium,
        Weight::Semibold,
        Weight::Bold,
        Weight::Heavy,
        Weight::Black,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Weight::UltraLight => "ultraLight",
            Weight::Thin => "thin",
            Weight::Light => "light",
            Weight::Regular => "regular",
            Weight::Medium => "medium",
            Weight::Semibold => "semibold",
            Weight::Bold => "bold",
            Weight::Heavy => "heavy",
            Weight::Black => "black",
        }
    }

    pub fn parse(s: &str) -> Option<Weight> {
        Weight::ALL.into_iter().find(|w| w.name() == s)
    }

    /// The CSS `font-weight` this weight maps to.
    pub fn css(self) -> u16 {
        match self {
            Weight::UltraLight => 100,
            Weight::Thin => 200,
            Weight::Light => 300,
            Weight::Regular => 400,
            Weight::Medium => 500,
            Weight::Semibold => 600,
            Weight::Bold => 700,
            Weight::Heavy => 800,
            Weight::Black => 900,
        }
    }
}

/// `"Family"`, `"Family/weight"` or `"Family/weight/italic"` in a manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub family: Family,
    pub weight: Weight,
    pub italic: bool,
}

impl Face {
    pub const fn new(family: Family, weight: Weight) -> Face {
        Face {
            family,
            weight,
            italic: false,
        }
    }

    /// `None` when the family is not allowed, the weight is unknown or there is trailing junk.
    pub fn parse(spec: &str) -> Option<Face> {
        let parts: Vec<&str> = spec.split('/').collect();
        if !(1..=3).contains(&parts.len()) {
            return None;
        }
        let family = Family::parse(parts[0])?;
        let mut face = Face::new(family, Weight::Regular);
        if parts.len() > 1 {
            face.weight = Weight::parse(parts[1])?;
        }
        if parts.len() > 2 {
            if parts[2] != "italic" {
                return None;
            }
            face.italic = true;
        }
        Some(face)
    }

    /// The canonical spelling: always `Family/weight`, then `/italic` when italic.
    pub fn spec(&self) -> String {
        let mut s = format!("{}/{}", self.family.name(), self.weight.name());
        if self.italic {
            s.push_str("/italic");
        }
        s
    }
}

impl Serialize for Face {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.spec())
    }
}

impl<'de> Deserialize<'de> for Face {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Face, D::Error> {
        let s = String::deserialize(d)?;
        Face::parse(&s).ok_or_else(|| D::Error::custom(format!("not a font face: {s}")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Caps {
    None,
    Upper,
    Small,
}

impl Caps {
    pub const ALL: [Caps; 3] = [Caps::None, Caps::Upper, Caps::Small];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Border {
    None,
    Solid,
    Dashed,
    Double,
}

impl Border {
    pub const ALL: [Border; 4] = [Border::None, Border::Solid, Border::Dashed, Border::Double];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Density {
    Compact,
    Regular,
    Airy,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Regular, Density::Airy];
}

impl Density {
    pub fn scale(self) -> f64 {
        match self {
            Density::Compact => 0.75,
            Density::Regular => 1.0,
            Density::Airy => 1.25,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Today,
    Meetings,
    Routines,
    Settings,
    Ask,
    Person,
    Time,
    Prep,
    Recap,
}

impl Role {
    pub const ALL: [Role; 9] = [
        Role::Today,
        Role::Meetings,
        Role::Routines,
        Role::Settings,
        Role::Ask,
        Role::Person,
        Role::Time,
        Role::Prep,
        Role::Recap,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Role::Today => "today",
            Role::Meetings => "meetings",
            Role::Routines => "routines",
            Role::Settings => "settings",
            Role::Ask => "ask",
            Role::Person => "person",
            Role::Time => "time",
            Role::Prep => "prep",
            Role::Recap => "recap",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HeaderBand {
    None,
    Rule,
    Block,
    #[serde(rename = "stripes-diagonal")]
    StripesDiagonal,
}

impl HeaderBand {
    pub const ALL: [HeaderBand; 4] = [
        HeaderBand::None,
        HeaderBand::Rule,
        HeaderBand::Block,
        HeaderBand::StripesDiagonal,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Divider {
    None,
    Hairline,
    Chalk,
    Double,
    Dotted,
}

impl Divider {
    pub const ALL: [Divider; 5] = [
        Divider::None,
        Divider::Hairline,
        Divider::Chalk,
        Divider::Double,
        Divider::Dotted,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Texture {
    None,
    Paper,
    Grain,
    Grid,
}

impl Texture {
    pub const ALL: [Texture; 4] = [Texture::None, Texture::Paper, Texture::Grain, Texture::Grid];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    Header,
    Background,
    Badge,
}

impl Placement {
    pub const ALL: [Placement; 3] = [Placement::Header, Placement::Background, Placement::Badge];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Archetype {
    Standard,
    Terminal,
    Glass,
    Field,
    Almanac,
    Reference,
}

impl Archetype {
    pub const ALL: [Archetype; 6] = [
        Archetype::Standard,
        Archetype::Terminal,
        Archetype::Glass,
        Archetype::Field,
        Archetype::Almanac,
        Archetype::Reference,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Motion {
    None,
    Subtle,
}

impl Motion {
    pub const ALL: [Motion; 2] = [Motion::None, Motion::Subtle];
}

/// The closed catalogue of strings a skin may reword (the manifest key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VoiceKey {
    GreetingMorning,
    GreetingAfternoon,
    GreetingEvening,
    UpNextHeader,
    UpcomingHeader,
    EarlierHeader,
    RecapHeader,
    EmptyDay,
    EmptyEarlier,
    AskPlaceholder,
    Tagline,
}

impl VoiceKey {
    pub const ALL: [VoiceKey; 11] = [
        VoiceKey::GreetingMorning,
        VoiceKey::GreetingAfternoon,
        VoiceKey::GreetingEvening,
        VoiceKey::UpNextHeader,
        VoiceKey::UpcomingHeader,
        VoiceKey::EarlierHeader,
        VoiceKey::RecapHeader,
        VoiceKey::EmptyDay,
        VoiceKey::EmptyEarlier,
        VoiceKey::AskPlaceholder,
        VoiceKey::Tagline,
    ];

    pub fn name(self) -> &'static str {
        match self {
            VoiceKey::GreetingMorning => "greetingMorning",
            VoiceKey::GreetingAfternoon => "greetingAfternoon",
            VoiceKey::GreetingEvening => "greetingEvening",
            VoiceKey::UpNextHeader => "upNextHeader",
            VoiceKey::UpcomingHeader => "upcomingHeader",
            VoiceKey::EarlierHeader => "earlierHeader",
            VoiceKey::RecapHeader => "recapHeader",
            VoiceKey::EmptyDay => "emptyDay",
            VoiceKey::EmptyEarlier => "emptyEarlier",
            VoiceKey::AskPlaceholder => "askPlaceholder",
            VoiceKey::Tagline => "tagline",
        }
    }

    pub fn limit(self) -> usize {
        match self {
            // 48: Whimbrel's own "Good afternoon, {name}" is 46 characters with a 30-character name.
            VoiceKey::GreetingMorning | VoiceKey::GreetingAfternoon | VoiceKey::GreetingEvening => {
                48
            }
            VoiceKey::AskPlaceholder => 40,
            VoiceKey::UpNextHeader
            | VoiceKey::UpcomingHeader
            | VoiceKey::EarlierHeader
            | VoiceKey::RecapHeader => 24,
            VoiceKey::EmptyDay | VoiceKey::EmptyEarlier => 80,
            VoiceKey::Tagline => 60,
        }
    }

    /// Non-empty, and within the limit once `{name}` is a 30-character name. Counted in grapheme
    /// clusters, as the Mac counts a Swift `String`.
    pub fn fits(self, value: &str) -> bool {
        !value.is_empty()
            && value
                .replace("{name}", &"x".repeat(MEASURED_NAME_LENGTH))
                .graphemes(true)
                .count()
                <= self.limit()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Typography {
    pub display: Face,
    pub body: Face,
    pub numerals: Face,
    pub label: Face,
    pub caps: Caps,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// Card corner radius.
    pub radius: f64,
    pub border: Border,
    #[serde(rename = "borderWidth")]
    pub border_width: f64,
    pub density: Density,
}

impl Shape {
    /// Inputs, editors and banners: two thirds of the card radius (12 -> 8).
    pub fn control_radius(&self) -> f64 {
        (self.radius * 2.0 / 3.0).round()
    }
}

/// The SF Symbol (a name from the icon table) each role draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Icons(pub BTreeMap<Role, String>);

impl Icons {
    pub fn get(&self, role: Role) -> &str {
        self.0
            .get(&role)
            .map(String::as_str)
            .unwrap_or("questionmark")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// A plain file name inside the skin's folder, never a path.
    pub asset: String,
    pub placement: Placement,
    /// Imagery marked text-free is never placed under text.
    #[serde(rename = "textFree")]
    pub text_free: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decoration {
    #[serde(rename = "headerBand")]
    pub header_band: HeaderBand,
    pub dividers: Divider,
    pub texture: Texture,
    pub imagery: Vec<Image>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Voice(pub BTreeMap<VoiceKey, String>);

impl Voice {
    pub fn text(&self, key: VoiceKey, name: &str) -> String {
        self.0
            .get(&key)
            .map(String::as_str)
            .unwrap_or("")
            .replace("{name}", name)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Today {
    pub archetype: Archetype,
    pub options: BTreeMap<String, String>,
}

/// A skin: the typography, shape, icons, decoration, voice and Today layout that restyle every screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skin {
    pub id: String,
    pub name: String,
    pub author: String,
    /// Palette ids; the first is the default colour option.
    pub palettes: Vec<String>,
    pub typography: Typography,
    pub shape: Shape,
    pub icons: Icons,
    pub decoration: Decoration,
    pub voice: Voice,
    pub today: Today,
    pub motion: Motion,
}

/// `[a-z0-9-]+`, the rule for skin and palette ids.
pub fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The Whimbrel skin: today's look. `skins/whimbrel.json` mirrors it (guarded by a test).
pub fn whimbrel_default() -> Skin {
    let icons = [
        (Role::Today, "sun.max"),
        (Role::Meetings, "person.2"),
        (Role::Routines, "bolt"),
        (Role::Settings, "gearshape"),
        (Role::Ask, "arrow.up.circle.fill"),
        (Role::Person, "person"),
        (Role::Time, "clock"),
        (Role::Prep, "doc.text"),
        (Role::Recap, "text.alignleft"),
    ];
    let voice = [
        (VoiceKey::GreetingMorning, "Good morning, {name}"),
        (VoiceKey::GreetingAfternoon, "Good afternoon, {name}"),
        (VoiceKey::GreetingEvening, "Good evening, {name}"),
        (VoiceKey::UpNextHeader, "Up next"),
        (VoiceKey::UpcomingHeader, "Today"),
        (VoiceKey::EarlierHeader, "Earlier"),
        (VoiceKey::RecapHeader, "Daily recap"),
        (VoiceKey::EmptyDay, "No meetings yet"),
        (VoiceKey::EmptyEarlier, "Nothing yet today"),
        (VoiceKey::AskPlaceholder, "Ask Whimbrel about your meetings"),
        (VoiceKey::Tagline, "Your meetings, remembered."),
    ];
    Skin {
        id: "whimbrel".into(),
        name: "Whimbrel".into(),
        author: "Whimbrel".into(),
        palettes: vec!["default".into(), "slate".into(), "sepia".into()],
        typography: Typography {
            display: Face::new(Family::NewYork, Weight::Semibold),
            body: Face::new(Family::SfPro, Weight::Regular),
            numerals: Face::new(Family::SfPro, Weight::Regular),
            label: Face::new(Family::SfPro, Weight::Regular),
            caps: Caps::Upper,
        },
        shape: Shape {
            radius: 12.0,
            border: Border::Solid,
            border_width: 1.0,
            density: Density::Regular,
        },
        icons: Icons(icons.into_iter().map(|(r, s)| (r, s.to_string())).collect()),
        decoration: Decoration {
            header_band: HeaderBand::None,
            dividers: Divider::None,
            texture: Texture::None,
            imagery: vec![],
        },
        voice: Voice(voice.into_iter().map(|(k, s)| (k, s.to_string())).collect()),
        today: Today {
            archetype: Archetype::Standard,
            options: BTreeMap::new(),
        },
        motion: Motion::None,
    }
}
