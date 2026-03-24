//! Bat species catalog — all species and families defined once, reused across books.
//!
//! Each constant represents a base species or family. Regional bat books
//! reference these via `BookEntryDef` and can override description/name.

use super::types::BatSpecies;

// ══════════════════════════════════════════════════════════════════════════════
// Family-level entries (used by the Global book)
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Jones & Barlow (2004) JEB: Scaling of echolocation call parameters
// - Jung et al. (2014) PMC: Molossidae call design
// - Shi et al. (2024) PMC: Correlated evolution body size & echolocation
// - Jones & Rayner (1989) Springer: Horseshoe bat foraging ecology
// - Collen (2012) BioOne: Rhinolophidae & Hipposideridae comparative ecology

pub const RHINOLOPHIDAE: BatSpecies = BatSpecies {
    id: "rhinolophidae",
    name: "Horseshoe bats",
    scientific_name: "",
    family: "Rhinolophidae",
    call_type: "CF",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 120_000.0,
    description: "Constant-frequency calls; species range ~30 kHz (large) to ~112 kHz (lesser horseshoe)",
    echolocates: true,
};

pub const HIPPOSIDERIDAE: BatSpecies = BatSpecies {
    id: "hipposideridae",
    name: "Roundleaf bats",
    scientific_name: "",
    family: "Hipposideridae",
    call_type: "CF",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 160_000.0,
    description: "CF calls; Cleotis percivalis reaches 212 kHz, the highest known bat frequency",
    echolocates: true,
};

pub const VESPERTILIONIDAE: BatSpecies = BatSpecies {
    id: "vespertilionidae",
    name: "Vesper bats",
    scientific_name: "",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 15_000.0,
    freq_hi_hz: 120_000.0,
    description: "Broadest family; FM sweeps; most species 20\u{2013}60 kHz peak",
    echolocates: true,
};

pub const MOLOSSIDAE: BatSpecies = BatSpecies {
    id: "molossidae",
    name: "Free-tailed bats",
    scientific_name: "",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 10_000.0,
    freq_hi_hz: 45_000.0,
    description: "Narrowband, long-duration QCF calls; 16\u{2013}44 kHz peak typical",
    echolocates: true,
};

pub const EMBALLONURIDAE: BatSpecies = BatSpecies {
    id: "emballonuridae",
    name: "Sheath-tailed bats",
    scientific_name: "",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 55_000.0,
    description: "Quasi-constant-frequency calls; some species sweep 40\u{2013}100 kHz",
    echolocates: true,
};

pub const PHYLLOSTOMIDAE: BatSpecies = BatSpecies {
    id: "phyllostomidae",
    name: "Leaf-nosed bats",
    scientific_name: "",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 120_000.0,
    description: "Low-intensity \"whispering\" bats; multi-harmonic FM calls",
    echolocates: true,
};

pub const MORMOOPIDAE: BatSpecies = BatSpecies {
    id: "mormoopidae",
    name: "Ghost-faced bats",
    scientific_name: "",
    family: "Mormoopidae",
    call_type: "CF-FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 65_000.0,
    description: "P. parnellii CF at ~63 kHz with FM sweep to ~54 kHz",
    echolocates: true,
};

pub const MINIOPTERIDAE: BatSpecies = BatSpecies {
    id: "miniopteridae",
    name: "Bent-winged bats",
    scientific_name: "",
    family: "Miniopteridae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 85_000.0,
    description: "FM-dominated calls; formerly classified within Vespertilionidae",
    echolocates: true,
};

pub const NYCTERIDAE: BatSpecies = BatSpecies {
    id: "nycteridae",
    name: "Slit-faced bats",
    scientific_name: "",
    family: "Nycteridae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 80_000.0,
    description: "Low-intensity, multi-harmonic FM calls emitted through nostrils",
    echolocates: true,
};

pub const MEGADERMATIDAE: BatSpecies = BatSpecies {
    id: "megadermatidae",
    name: "False vampires",
    scientific_name: "",
    family: "Megadermatidae",
    call_type: "FM",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 110_000.0,
    description: "Low-intensity broadband FM; large carnivorous bats",
    echolocates: true,
};

pub const PTEROPODIDAE: BatSpecies = BatSpecies {
    id: "pteropodidae",
    name: "Fruit bats",
    scientific_name: "",
    family: "Pteropodidae",
    call_type: "clicks",
    freq_lo_hz: 10_000.0,
    freq_hi_hz: 100_000.0,
    description: "Most don't echolocate; Rousettus uses tongue clicks for cave navigation",
    echolocates: false,
};

// ══════════════════════════════════════════════════════════════════════════════
// Species: Victoria, Australia
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Batica: Microbat Call Identification Assistant (Bayside, VIC)
// - SWIFFT: Insectivorous bats of Victoria
// - Milne (2002): Key to the Bat Calls of the Top End of the NT
// - Wikipedia: List of bats of Australia
// - Museums Victoria, Atlas of Living Australia

pub const CHALINOLOBUS_GOULDII: BatSpecies = BatSpecies {
    id: "chalinolobus_gouldii",
    name: "Gould's Wattled Bat",
    scientific_name: "Chalinolobus gouldii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 34_000.0,
    description: "Widespread and abundant across Australia. Roosts in tree hollows, buildings, and bat boxes. Alternating call frequencies are distinctive.",
    echolocates: true,
};

pub const CHALINOLOBUS_MORIO: BatSpecies = BatSpecies {
    id: "chalinolobus_morio",
    name: "Chocolate Wattled Bat",
    scientific_name: "Chalinolobus morio",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 48_000.0,
    freq_hi_hz: 53_000.0,
    description: "Common across southern Australia. Small, dark bat roosting in tree hollows and buildings. Higher frequency calls than Gould's Wattled Bat.",
    echolocates: true,
};

pub const NYCTOPHILUS_GEOFFROYI: BatSpecies = BatSpecies {
    id: "nyctophilus_geoffroyi",
    name: "Lesser Long-eared Bat",
    scientific_name: "Nyctophilus geoffroyi",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 80_000.0,
    description: "Australia's most widespread bat. Very quiet, broadband FM calls; often difficult to detect acoustically. Gleaning insectivore with large ears.",
    echolocates: true,
};

pub const AUSTRONOMUS_AUSTRALIS: BatSpecies = BatSpecies {
    id: "austronomus_australis",
    name: "White-striped Free-tailed Bat",
    scientific_name: "Austronomus australis",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 10_000.0,
    freq_hi_hz: 15_000.0,
    description: "Australia's largest insectivorous bat. Loud, low-frequency calls audible to some humans. Fast, high-flying open-air forager.",
    echolocates: true,
};

pub const VESPADELUS_VULTURNUS: BatSpecies = BatSpecies {
    id: "vespadelus_vulturnus",
    name: "Little Forest Bat",
    scientific_name: "Vespadelus vulturnus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 53_000.0,
    description: "One of Australia's smallest bats (~4 g). Common in forests and urban areas. High-frequency calls.",
    echolocates: true,
};

pub const VESPADELUS_REGULUS: BatSpecies = BatSpecies {
    id: "vespadelus_regulus",
    name: "Southern Forest Bat",
    scientific_name: "Vespadelus regulus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 55_000.0,
    description: "Small forest bat found across southern Australia. Roosts in tree hollows. Call frequency overlaps with Little Forest Bat.",
    echolocates: true,
};

pub const NYCTOPHILUS_GOULDI: BatSpecies = BatSpecies {
    id: "nyctophilus_gouldi",
    name: "Gould's Long-eared Bat",
    scientific_name: "Nyctophilus gouldi",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 80_000.0,
    description: "Common in forests of eastern Australia. Very quiet calls, similar to Lesser Long-eared Bat. Distinguished by larger size and habitat preference.",
    echolocates: true,
};

pub const VESPADELUS_DARLINGTONI: BatSpecies = BatSpecies {
    id: "vespadelus_darlingtoni",
    name: "Large Forest Bat",
    scientific_name: "Vespadelus darlingtoni",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 38_000.0,
    freq_hi_hz: 46_000.0,
    description: "Largest Vespadelus species. Found in wet and dry forests of south-eastern Australia including Tasmania.",
    echolocates: true,
};

pub const MINIOPTERUS_ORIANAE_OCEANENSIS: BatSpecies = BatSpecies {
    id: "miniopterus_orianae_oceanensis",
    name: "Eastern Bent-wing Bat",
    scientific_name: "Miniopterus orianae oceanensis",
    family: "Miniopteridae",
    call_type: "FM",
    freq_lo_hz: 43_000.0,
    freq_hi_hz: 48_000.0,
    description: "Cave-roosting bat found along eastern Australia. Fast, agile flier.",
    echolocates: true,
};

pub const OZIMOPS_PLANICEPS: BatSpecies = BatSpecies {
    id: "ozimops_planiceps",
    name: "Southern Free-tailed Bat",
    scientific_name: "Ozimops planiceps",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 29_000.0,
    description: "Small free-tailed bat of south-eastern Australia. Roosts in tree hollows and buildings. Rapid, direct flight.",
    echolocates: true,
};

pub const OZIMOPS_RIDEI: BatSpecies = BatSpecies {
    id: "ozimops_ridei",
    name: "Ride's Free-tailed Bat",
    scientific_name: "Ozimops ridei",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 35_000.0,
    description: "Widespread across eastern Australian coasts. Similar to Southern Free-tailed Bat but slightly higher frequency calls.",
    echolocates: true,
};

pub const FALSISTRELLUS_TASMANIENSIS: BatSpecies = BatSpecies {
    id: "falsistrellus_tasmaniensis",
    name: "Eastern Falsistrelle",
    scientific_name: "Falsistrellus tasmaniensis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 34_000.0,
    freq_hi_hz: 39_000.0,
    description: "Large vesper bat of south-eastern forests. Roosts in tree hollows. Vulnerable (IUCN). Distinctive mid-range frequency calls.",
    echolocates: true,
};

pub const SCOTOREPENS_ORION: BatSpecies = BatSpecies {
    id: "scotorepens_orion",
    name: "Eastern Broad-nosed Bat",
    scientific_name: "Scotorepens orion",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 34_500.0,
    freq_hi_hz: 37_500.0,
    description: "Robust bat of south-eastern coastal forests. Narrow frequency range distinctive. Roosts in tree hollows.",
    echolocates: true,
};

pub const SCOTOREPENS_BALSTONI: BatSpecies = BatSpecies {
    id: "scotorepens_balstoni",
    name: "Inland Broad-nosed Bat",
    scientific_name: "Scotorepens balstoni",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 34_000.0,
    description: "Widespread across inland Australia. Found in drier regions. Similar frequency to Gould's Wattled Bat.",
    echolocates: true,
};

pub const MYOTIS_MACROPUS: BatSpecies = BatSpecies {
    id: "myotis_macropus",
    name: "Large-footed Myotis",
    scientific_name: "Myotis macropus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 80_000.0,
    description: "Australia's only fishing bat. Trawls water surfaces with large feet. Found near rivers, lakes, and dams. Very quiet calls.",
    echolocates: true,
};

pub const SACCOLAIMUS_FLAVIVENTRIS: BatSpecies = BatSpecies {
    id: "saccolaimus_flaviventris",
    name: "Yellow-bellied Sheathtail Bat",
    scientific_name: "Saccolaimus flaviventris",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 17_500.0,
    freq_hi_hz: 22_500.0,
    description: "Large, fast-flying bat with glossy black fur and yellow belly. Migratory. High-altitude forager.",
    echolocates: true,
};

pub const RHINOLOPHUS_MEGAPHYLLUS: BatSpecies = BatSpecies {
    id: "rhinolophus_megaphyllus",
    name: "Eastern Horseshoe Bat",
    scientific_name: "Rhinolophus megaphyllus",
    family: "Rhinolophidae",
    call_type: "CF",
    freq_lo_hz: 67_000.0,
    freq_hi_hz: 71_000.0,
    description: "Constant-frequency echolocation using distinctive horseshoe-shaped noseleaf. Cave-roosting. Found in forests of eastern Australia.",
    echolocates: true,
};

pub const PTEROPUS_POLIOCEPHALUS: BatSpecies = BatSpecies {
    id: "pteropus_poliocephalus",
    name: "Grey-headed Flying-fox",
    scientific_name: "Pteropus poliocephalus",
    family: "Pteropodidae",
    call_type: "none",
    freq_lo_hz: 0.0,
    freq_hi_hz: 0.0,
    description: "Australia's largest bat (wingspan ~1 m). Does not echolocate. Camps in colonies along waterways. Vulnerable (EPBC Act). Pollinator and seed disperser.",
    echolocates: false,
};

pub const NYCTOPHILUS_MAJOR: BatSpecies = BatSpecies {
    id: "nyctophilus_major",
    name: "Greater Long-eared Bat",
    scientific_name: "Nyctophilus major",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 65_000.0,
    description: "Formerly N. timoriensis. Restricted to drier woodlands. Very quiet gleaning calls. Vulnerable.",
    echolocates: true,
};

pub const VESPADELUS_BAVERSTOCKI: BatSpecies = BatSpecies {
    id: "vespadelus_baverstocki",
    name: "Inland Forest Bat",
    scientific_name: "Vespadelus baverstocki",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 50_000.0,
    description: "Small bat of inland Australia. Restricted to semi-arid regions.",
    echolocates: true,
};

pub const SCOTEANAX_RUEPPELLII: BatSpecies = BatSpecies {
    id: "scoteanax_rueppellii",
    name: "Greater Broad-nosed Bat",
    scientific_name: "Scoteanax rueppellii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 38_000.0,
    description: "Large, robust bat of eastern coastal forests. Aggressive predator of large insects and small vertebrates.",
    echolocates: true,
};

pub const MINIOPTERUS_ORIANAE_BASSANII: BatSpecies = BatSpecies {
    id: "miniopterus_orianae_bassanii",
    name: "Southern Bent-wing Bat",
    scientific_name: "Miniopterus orianae bassanii",
    family: "Miniopteridae",
    call_type: "FM",
    freq_lo_hz: 43_000.0,
    freq_hi_hz: 48_000.0,
    description: "Critically Endangered (EPBC Act). Dependent on a single maternity cave. Population critically low.",
    echolocates: true,
};

pub const NYCTOPHILUS_CORBENI: BatSpecies = BatSpecies {
    id: "nyctophilus_corbeni",
    name: "South-eastern Long-eared Bat",
    scientific_name: "Nyctophilus corbeni",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 65_000.0,
    description: "Vulnerable (EPBC Act). Extremely rare; restricted to northwest woodlands.",
    echolocates: true,
};

pub const PTEROPUS_SCAPULATUS: BatSpecies = BatSpecies {
    id: "pteropus_scapulatus",
    name: "Little Red Flying-fox",
    scientific_name: "Pteropus scapulatus",
    family: "Pteropodidae",
    call_type: "none",
    freq_lo_hz: 0.0,
    freq_hi_hz: 0.0,
    description: "Does not echolocate. Nomadic, following eucalypt flowering. Seasonal visitor to southern regions.",
    echolocates: false,
};

// ══════════════════════════════════════════════════════════════════════════════
// Species: Europe
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Dietz, Helversen & Nill (2009): Bats of Britain, Europe and Northwest Africa
// - Russ (2012): British Bat Calls: A Guide to Species Identification
// - Barataud (2015): Acoustic Ecology of European Bats
// - Middleton, Froud & French (2014): Social Calls of the Bats of Britain and Ireland

pub const PIPISTRELLUS_PIPISTRELLUS: BatSpecies = BatSpecies {
    id: "pipistrellus_pipistrellus",
    name: "Common Pipistrelle",
    scientific_name: "Pipistrellus pipistrellus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 42_000.0,
    freq_hi_hz: 51_000.0,
    description: "Europe's most abundant bat. Characteristic frequency ~45 kHz separates it from soprano pipistrelle. Roosts in buildings; forages along edges and over water.",
    echolocates: true,
};

pub const PIPISTRELLUS_PYGMAEUS: BatSpecies = BatSpecies {
    id: "pipistrellus_pygmaeus",
    name: "Soprano Pipistrelle",
    scientific_name: "Pipistrellus pygmaeus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 51_000.0,
    freq_hi_hz: 60_000.0,
    description: "Cryptic species split from common pipistrelle in 1999. Characteristic frequency ~55 kHz. Strongly associated with riparian habitats.",
    echolocates: true,
};

pub const PIPISTRELLUS_NATHUSII: BatSpecies = BatSpecies {
    id: "pipistrellus_nathusii",
    name: "Nathusius' Pipistrelle",
    scientific_name: "Pipistrellus nathusii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 42_000.0,
    description: "Long-distance migrant; travels up to 2,000 km. Characteristic frequency ~38 kHz. Favours wetlands and riparian woodland.",
    echolocates: true,
};

pub const PIPISTRELLUS_KUHLII: BatSpecies = BatSpecies {
    id: "pipistrellus_kuhlii",
    name: "Kuhl's Pipistrelle",
    scientific_name: "Pipistrellus kuhlii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 45_000.0,
    description: "Expanding northward across Europe. Characteristic frequency ~40 kHz. Common around buildings and street lights in Mediterranean regions.",
    echolocates: true,
};

pub const MYOTIS_DAUBENTONII: BatSpecies = BatSpecies {
    id: "myotis_daubentonii",
    name: "Daubenton's Bat",
    scientific_name: "Myotis daubentonii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 32_000.0,
    freq_hi_hz: 85_000.0,
    description: "Forages low over calm water, trawling insects from the surface. Steep FM sweeps. Often seen along canals and rivers at dusk.",
    echolocates: true,
};

pub const MYOTIS_NATTERERI: BatSpecies = BatSpecies {
    id: "myotis_nattereri",
    name: "Natterer's Bat",
    scientific_name: "Myotis nattereri",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 23_000.0,
    freq_hi_hz: 115_000.0,
    description: "Very broadband FM calls with prominent harmonics. Gleaning specialist, picks prey from foliage and walls. Roosts in old buildings and tree holes.",
    echolocates: true,
};

pub const MYOTIS_MYSTACINUS: BatSpecies = BatSpecies {
    id: "myotis_mystacinus",
    name: "Whiskered Bat",
    scientific_name: "Myotis mystacinus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 32_000.0,
    freq_hi_hz: 80_000.0,
    description: "Small Myotis often found in villages and woodland edges. Very similar in call and appearance to Brandt's bat; confirmed by handling or genetics.",
    echolocates: true,
};

pub const MYOTIS_BRANDTII: BatSpecies = BatSpecies {
    id: "myotis_brandtii",
    name: "Brandt's Bat",
    scientific_name: "Myotis brandtii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 80_000.0,
    description: "Closely related to whiskered bat; prefers mature woodland. Slightly lower frequency calls. Identified reliably only by dentition or genetics.",
    echolocates: true,
};

pub const MYOTIS_MYOTIS: BatSpecies = BatSpecies {
    id: "myotis_myotis",
    name: "Greater Mouse-eared Bat",
    scientific_name: "Myotis myotis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 80_000.0,
    description: "One of Europe's largest vespertilionids. Ground-gleaning specialist hunting beetles in short grass and forest floors. Large nursery colonies in roofs and caves.",
    echolocates: true,
};

pub const MYOTIS_BECHSTEINII: BatSpecies = BatSpecies {
    id: "myotis_bechsteinii",
    name: "Bechstein's Bat",
    scientific_name: "Myotis bechsteinii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 100_000.0,
    description: "Rare woodland specialist with very quiet, broadband calls. Indicator species for old-growth forest. Roosts in tree holes; rarely found in buildings.",
    echolocates: true,
};

pub const MYOTIS_DASYCNEME: BatSpecies = BatSpecies {
    id: "myotis_dasycneme",
    name: "Pond Bat",
    scientific_name: "Myotis dasycneme",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 65_000.0,
    description: "Larger relative of Daubenton's bat. Trawls over broad lakes and slow rivers. Vulnerable across most of its range in NW Europe.",
    echolocates: true,
};

pub const NYCTALUS_NOCTULA: BatSpecies = BatSpecies {
    id: "nyctalus_noctula",
    name: "Common Noctule",
    scientific_name: "Nyctalus noctula",
    family: "Vespertilionidae",
    call_type: "QCF",
    freq_lo_hz: 18_000.0,
    freq_hi_hz: 25_000.0,
    description: "Large, fast-flying bat. Loud, narrowband calls audible on bat detectors at distance. Roosts in tree holes; one of the first species to emerge at dusk.",
    echolocates: true,
};

pub const NYCTALUS_LEISLERI: BatSpecies = BatSpecies {
    id: "nyctalus_leisleri",
    name: "Leisler's Bat",
    scientific_name: "Nyctalus leisleri",
    family: "Vespertilionidae",
    call_type: "QCF",
    freq_lo_hz: 22_000.0,
    freq_hi_hz: 30_000.0,
    description: "Smaller noctule with slightly higher frequency calls. Fast open-air forager. Migratory in parts of its range. Common in Ireland.",
    echolocates: true,
};

pub const NYCTALUS_LASIOPTERUS: BatSpecies = BatSpecies {
    id: "nyctalus_lasiopterus",
    name: "Greater Noctule",
    scientific_name: "Nyctalus lasiopterus",
    family: "Vespertilionidae",
    call_type: "QCF",
    freq_lo_hz: 14_000.0,
    freq_hi_hz: 20_000.0,
    description: "Europe's largest bat. Occasionally catches small birds in flight during nocturnal migration. Very low-frequency calls. Rare across its range.",
    echolocates: true,
};

pub const EPTESICUS_SEROTINUS: BatSpecies = BatSpecies {
    id: "eptesicus_serotinus",
    name: "Serotine",
    scientific_name: "Eptesicus serotinus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 22_000.0,
    freq_hi_hz: 55_000.0,
    description: "Large bat with broad FM sweeps. One of the last to emerge, often foraging along tree lines and around street lights. Roosts almost exclusively in buildings.",
    echolocates: true,
};

pub const EPTESICUS_NILSSONII: BatSpecies = BatSpecies {
    id: "eptesicus_nilssonii",
    name: "Northern Bat",
    scientific_name: "Eptesicus nilssonii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 45_000.0,
    description: "The world's northernmost bat, found above the Arctic Circle. Tolerates cold climates. Common in Scandinavia and mountain regions of central Europe.",
    echolocates: true,
};

pub const PLECOTUS_AURITUS: BatSpecies = BatSpecies {
    id: "plecotus_auritus",
    name: "Brown Long-eared Bat",
    scientific_name: "Plecotus auritus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 85_000.0,
    description: "Iconic enormous ears. Very quiet, broadband calls; often called a \"whispering bat\". Gleaning specialist in woodland. Roosts in buildings and tree holes.",
    echolocates: true,
};

pub const PLECOTUS_AUSTRIACUS: BatSpecies = BatSpecies {
    id: "plecotus_austriacus",
    name: "Grey Long-eared Bat",
    scientific_name: "Plecotus austriacus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 22_000.0,
    freq_hi_hz: 50_000.0,
    description: "Prefers warmer lowland areas. Quieter and slightly lower frequency than brown long-eared. Difficult to distinguish visually; confirmed by tragus shape.",
    echolocates: true,
};

pub const BARBASTELLA_BARBASTELLUS: BatSpecies = BatSpecies {
    id: "barbastella_barbastellus",
    name: "Barbastelle",
    scientific_name: "Barbastella barbastellus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 45_000.0,
    description: "Distinctive flat face with upturned nose. Alternating call frequencies (~32 and ~34 kHz). Specialist moth-hunter; forest dependent. Rare across most of its range.",
    echolocates: true,
};

pub const VESPERTILIO_MURINUS: BatSpecies = BatSpecies {
    id: "vespertilio_murinus",
    name: "Parti-coloured Bat",
    scientific_name: "Vespertilio murinus",
    family: "Vespertilionidae",
    call_type: "QCF",
    freq_lo_hz: 22_000.0,
    freq_hi_hz: 30_000.0,
    description: "Striking frosted fur. Alternating call frequencies. Migratory; roosts in high-rise buildings. Males produce audible courtship calls from roost entrances.",
    echolocates: true,
};

pub const RHINOLOPHUS_FERRUMEQUINUM: BatSpecies = BatSpecies {
    id: "rhinolophus_ferrumequinum",
    name: "Greater Horseshoe Bat",
    scientific_name: "Rhinolophus ferrumequinum",
    family: "Rhinolophidae",
    call_type: "CF",
    freq_lo_hz: 78_000.0,
    freq_hi_hz: 84_000.0,
    description: "Europe's largest horseshoe bat. Constant-frequency call at ~83 kHz. Hunts large beetles and moths in flight. Roosts in caves, mines, and old buildings.",
    echolocates: true,
};

pub const RHINOLOPHUS_HIPPOSIDEROS: BatSpecies = BatSpecies {
    id: "rhinolophus_hipposideros",
    name: "Lesser Horseshoe Bat",
    scientific_name: "Rhinolophus hipposideros",
    family: "Rhinolophidae",
    call_type: "CF",
    freq_lo_hz: 105_000.0,
    freq_hi_hz: 115_000.0,
    description: "One of Europe's smallest bats (~5 g). CF call at ~110 kHz. Forages close to vegetation in sheltered valleys. Very sensitive to disturbance at roost sites.",
    echolocates: true,
};

pub const RHINOLOPHUS_EURYALE: BatSpecies = BatSpecies {
    id: "rhinolophus_euryale",
    name: "Mediterranean Horseshoe Bat",
    scientific_name: "Rhinolophus euryale",
    family: "Rhinolophidae",
    call_type: "CF",
    freq_lo_hz: 100_000.0,
    freq_hi_hz: 108_000.0,
    description: "Medium-sized horseshoe bat. CF call at ~104 kHz. Restricted to Mediterranean and warm-temperate zones. Cave-dwelling; large colony roosts.",
    echolocates: true,
};

pub const TADARIDA_TENIOTIS: BatSpecies = BatSpecies {
    id: "tadarida_teniotis",
    name: "European Free-tailed Bat",
    scientific_name: "Tadarida teniotis",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 10_000.0,
    freq_hi_hz: 18_000.0,
    description: "Europe's only free-tailed bat. Loud, low-frequency calls audible to humans. Fast, high-altitude forager. Roosts in cliff crevices and tall buildings.",
    echolocates: true,
};

pub const MINIOPTERUS_SCHREIBERSII: BatSpecies = BatSpecies {
    id: "miniopterus_schreibersii",
    name: "Common Bent-wing Bat",
    scientific_name: "Miniopterus schreibersii",
    family: "Miniopteridae",
    call_type: "FM",
    freq_lo_hz: 47_000.0,
    freq_hi_hz: 57_000.0,
    description: "Fast, agile cave-dweller found across southern Europe. Long, narrow wings for sustained flight. Large colonies; sensitive to cave disturbance.",
    echolocates: true,
};

// ── Additional UK species ────────────────────────────────────────────────────

pub const MYOTIS_ALCATHOE: BatSpecies = BatSpecies {
    id: "myotis_alcathoe",
    name: "Alcathoe Bat",
    scientific_name: "Myotis alcathoe",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 105_000.0,
    description: "Tiny Myotis first described in 2001, confirmed breeding in the UK in 2010. FM calls sweep from ~100 kHz down to ~40 kHz with peak energy around 50–55 kHz. Closely resembles whiskered and Brandt's bats but smaller with shorter ears.",
    echolocates: true,
};

pub const HYPSUGO_SAVII: BatSpecies = BatSpecies {
    id: "hypsugo_savii",
    name: "Savi's Pipistrelle",
    scientific_name: "Hypsugo savii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 42_000.0,
    description: "Medium pipistrelle with shallow FM sweep ending in a quasi-constant-frequency tail around 32–34 kHz. Primarily Mediterranean; increasingly recorded as a vagrant in southern England.",
    echolocates: true,
};

// ══════════════════════════════════════════════════════════════════════════════
// Species: Costa Rica
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Jung et al. (2007): Echolocation calls in Central American emballonurids
// - Leiser-Miller & Santana (2021): Functional differences in phyllostomid echolocation (Costa Rica data)
// - Gessinger et al. (2019): Unusual CF-FM echolocation of Lonchorhina aurita
// - Schnitzler et al.: Fishing and echolocation of Noctilio leporinus
// - Kalko et al.: Echolocation and foraging of Noctilio albiventris
// - Zamora-Gutierrez et al. (2016): Acoustic identification of Mexican bats
// - Rydell et al. (2002): Acoustic identification of Yucatan bats
// - Yoh et al. (2020): Echolocation of Amazonian phyllostomid bats

// ── Noctilionidae ──────────────────────────────────────────────────────────

pub const NOCTILIO_LEPORINUS: BatSpecies = BatSpecies {
    id: "noctilio_leporinus",
    name: "Greater Bulldog Bat",
    scientific_name: "Noctilio leporinus",
    family: "Noctilionidae",
    call_type: "CF-FM",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 56_000.0,
    description: "Large fishing bat. Long CF component at 53\u{2013}56 kHz followed by FM sweep. Forages over water; one of few New World bats with long CF calls.",
    echolocates: true,
};

pub const NOCTILIO_ALBIVENTRIS: BatSpecies = BatSpecies {
    id: "noctilio_albiventris",
    name: "Lesser Bulldog Bat",
    scientific_name: "Noctilio albiventris",
    family: "Noctilionidae",
    call_type: "CF-FM",
    freq_lo_hz: 57_000.0,
    freq_hi_hz: 75_000.0,
    description: "Smaller than N. leporinus with higher CF at ~75 kHz. Trawls insects from water surfaces. CF-FM call structure similar to greater bulldog bat.",
    echolocates: true,
};

// ── Emballonuridae ─────────────────────────────────────────────────────────

pub const SACCOPTERYX_BILINEATA: BatSpecies = BatSpecies {
    id: "saccopteryx_bilineata",
    name: "Greater Sac-winged Bat",
    scientific_name: "Saccopteryx bilineata",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 43_000.0,
    freq_hi_hz: 49_000.0,
    description: "Alternates between ~45 and ~48 kHz when foraging. Most energy in 2nd harmonic. Well-studied species with vocal learning. Distinctive white dorsal stripes.",
    echolocates: true,
};

pub const SACCOPTERYX_LEPTURA: BatSpecies = BatSpecies {
    id: "saccopteryx_leptura",
    name: "Lesser Sac-winged Bat",
    scientific_name: "Saccopteryx leptura",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 47_000.0,
    freq_hi_hz: 53_000.0,
    description: "Higher frequency than S. bilineata. Similar multiharmonic QCF structure. Most energy in 2nd harmonic. Thinner dorsal stripes.",
    echolocates: true,
};

pub const RHYNCHONYCTERIS_NASO: BatSpecies = BatSpecies {
    id: "rhynchonycteris_naso",
    name: "Proboscis Bat",
    scientific_name: "Rhynchonycteris naso",
    family: "Emballonuridae",
    call_type: "CF-FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 100_000.0,
    description: "Tiny bat roosting along rivers. Short CF at ~47 kHz plus FM downsweep. Peak drops from ~100 kHz (search) to ~67 kHz (buzz) during prey pursuit.",
    echolocates: true,
};

pub const CORMURA_BREVIROSTRIS: BatSpecies = BatSpecies {
    id: "cormura_brevirostris",
    name: "Wagner's Sac-winged Bat",
    scientific_name: "Cormura brevirostris",
    family: "Emballonuridae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 90_000.0,
    description: "Unusual: most energy in 5th harmonic (~68 kHz). Forest-interior forager. Multiharmonic calls with shallow FM sweeps.",
    echolocates: true,
};

pub const BALANTIOPTERYX_PLICATA: BatSpecies = BatSpecies {
    id: "balantiopteryx_plicata",
    name: "Gray Sac-winged Bat",
    scientific_name: "Balantiopteryx plicata",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 22_000.0,
    freq_hi_hz: 50_000.0,
    description: "Open-area forager. Long narrowband QCF (14\u{2013}20 ms) plus short broadband component. Displays jamming avoidance by shifting peak frequency in groups.",
    echolocates: true,
};

pub const PEROPTERYX_MACROTIS: BatSpecies = BatSpecies {
    id: "peropteryx_macrotis",
    name: "Lesser Dog-like Bat",
    scientific_name: "Peropteryx macrotis",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 50_000.0,
    description: "Multiharmonic QCF with most energy in 2nd harmonic at ~40 kHz. Higher frequency than P. kappleri.",
    echolocates: true,
};

pub const PEROPTERYX_KAPPLERI: BatSpecies = BatSpecies {
    id: "peropteryx_kappleri",
    name: "Greater Dog-like Bat",
    scientific_name: "Peropteryx kappleri",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 40_000.0,
    description: "Lower frequency (~32 kHz) than P. macrotis, consistent with larger body size. Open-space forager. Most energy in 2nd harmonic.",
    echolocates: true,
};

pub const DICLIDURUS_ALBUS: BatSpecies = BatSpecies {
    id: "diclidurus_albus",
    name: "Northern Ghost Bat",
    scientific_name: "Diclidurus albus",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 22_000.0,
    freq_hi_hz: 27_000.0,
    description: "Large emballonurid with distinctive white fur. Narrowband QCF at ~24 kHz. Long inter-pulse intervals. Rarely encountered.",
    echolocates: true,
};

// ── Mormoopidae ────────────────────────────────────────────────────────────

pub const PTERONOTUS_MESOAMERICANUS: BatSpecies = BatSpecies {
    id: "pteronotus_mesoamericanus",
    name: "Mesoamerican Mustached Bat",
    scientific_name: "Pteronotus mesoamericanus",
    family: "Mormoopidae",
    call_type: "CF-FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 120_000.0,
    description: "The ONLY New World high-duty-cycle echolocator. Long CF at ~61 kHz (2nd harmonic) with Doppler compensation. 4\u{2013}5 harmonics. Formerly P. parnellii.",
    echolocates: true,
};

pub const PTERONOTUS_DAVYI: BatSpecies = BatSpecies {
    id: "pteronotus_davyi",
    name: "Davy's Naked-backed Bat",
    scientific_name: "Pteronotus davyi",
    family: "Mormoopidae",
    call_type: "CF-FM",
    freq_lo_hz: 51_000.0,
    freq_hi_hz: 68_000.0,
    description: "CF-FM with short CF at ~67 kHz, FM sweep down to ~51 kHz. Shorter calls than P. mesoamericanus. Most energy in 2nd harmonic. Wing membranes fused across back.",
    echolocates: true,
};

pub const PTERONOTUS_GYMNONOTUS: BatSpecies = BatSpecies {
    id: "pteronotus_gymnonotus",
    name: "Big Naked-backed Bat",
    scientific_name: "Pteronotus gymnonotus",
    family: "Mormoopidae",
    call_type: "CF-FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 65_000.0,
    description: "CF component at ~54\u{2013}57 kHz. Similar structure to P. davyi but lower frequency, consistent with larger body size.",
    echolocates: true,
};

pub const PTERONOTUS_PERSONATUS: BatSpecies = BatSpecies {
    id: "pteronotus_personatus",
    name: "Wagner's Mustached Bat",
    scientific_name: "Pteronotus personatus",
    family: "Mormoopidae",
    call_type: "CF-FM",
    freq_lo_hz: 62_000.0,
    freq_hi_hz: 83_000.0,
    description: "Highest frequency Pteronotus in Costa Rica (excl. P. mesoamericanus harmonics). Initial CF ~83 kHz, terminal CF ~68 kHz. Shows Doppler-shift compensation.",
    echolocates: true,
};

pub const MORMOOPS_MEGALOPHYLLA: BatSpecies = BatSpecies {
    id: "mormoops_megalophylla",
    name: "Ghost-faced Bat",
    scientific_name: "Mormoops megalophylla",
    family: "Mormoopidae",
    call_type: "QCF",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 70_000.0,
    description: "Bizarre facial leaf. Fundamental ~30 kHz suppressed; only 2nd harmonic (~67 kHz) typically recorded. Cave-roosting; large colonies.",
    echolocates: true,
};

// ── Phyllostomidae: Phyllostominae (insectivores/carnivores) ───────────────

pub const VAMPYRUM_SPECTRUM: BatSpecies = BatSpecies {
    id: "vampyrum_spectrum",
    name: "Spectral Bat",
    scientific_name: "Vampyrum spectrum",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 90_000.0,
    description: "Largest bat in the Americas (wingspan ~1 m). Low-intensity multiharmonic FM. Carnivorous: preys on birds and other bats. Very difficult to detect acoustically.",
    echolocates: true,
};

pub const CHROTOPTERUS_AURITUS: BatSpecies = BatSpecies {
    id: "chrotopterus_auritus",
    name: "Big-eared Woolly Bat",
    scientific_name: "Chrotopterus auritus",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 67_000.0,
    freq_hi_hz: 88_000.0,
    description: "Carnivorous gleaner. Peak ~77 kHz, higher than predicted for body size. Short FM sweeps (0.8\u{2013}1.4 ms). Low-intensity; hard to detect beyond a few meters.",
    echolocates: true,
};

pub const PHYLLOSTOMUS_HASTATUS: BatSpecies = BatSpecies {
    id: "phyllostomus_hastatus",
    name: "Greater Spear-nosed Bat",
    scientific_name: "Phyllostomus hastatus",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 34_000.0,
    freq_hi_hz: 60_000.0,
    description: "Large omnivore. One of the lowest-frequency phyllostomids (~47 kHz peak). Multiharmonic FM. Forms harem groups in caves and hollow trees.",
    echolocates: true,
};

pub const PHYLLOSTOMUS_DISCOLOR: BatSpecies = BatSpecies {
    id: "phyllostomus_discolor",
    name: "Pale Spear-nosed Bat",
    scientific_name: "Phyllostomus discolor",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 80_000.0,
    description: "Omnivore with lower frequency than most phyllostomids (~55 kHz peak). Best hearing sensitivity at 20 kHz. Low-intensity whispering calls.",
    echolocates: true,
};

pub const LONCHORHINA_AURITA: BatSpecies = BatSpecies {
    id: "lonchorhina_aurita",
    name: "Tomes's Sword-nosed Bat",
    scientific_name: "Lonchorhina aurita",
    family: "Phyllostomidae",
    call_type: "CF-FM",
    freq_lo_hz: 38_000.0,
    freq_hi_hz: 50_000.0,
    description: "UNIQUE among phyllostomids: uses CF-FM calls like mormoopids. Long CF at ~45 kHz (3rd harmonic dominant). Longest calls of any phyllostomid (up to 8.7 ms).",
    echolocates: true,
};

pub const TRACHOPS_CIRRHOSUS: BatSpecies = BatSpecies {
    id: "trachops_cirrhosus",
    name: "Fringe-lipped Bat",
    scientific_name: "Trachops cirrhosus",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 100_000.0,
    description: "Famous frog-eating bat. Listens for frog mating calls to locate prey. Low-intensity multiharmonic FM (~70 kHz peak). Hard to detect acoustically.",
    echolocates: true,
};

pub const MICRONYCTERIS_MICROTIS: BatSpecies = BatSpecies {
    id: "micronycteris_microtis",
    name: "Common Big-eared Bat",
    scientific_name: "Micronycteris microtis",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 120_000.0,
    description: "Gleaning insectivore in dense understory. Can detect silent, motionless prey. Very short broadband calls (0.3\u{2013}1 ms) at high frequency (~90\u{2013}100 kHz). Ultra-low intensity.",
    echolocates: true,
};

pub const MICRONYCTERIS_HIRSUTA: BatSpecies = BatSpecies {
    id: "micronycteris_hirsuta",
    name: "Hairy Big-eared Bat",
    scientific_name: "Micronycteris hirsuta",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 51_000.0,
    freq_hi_hz: 90_000.0,
    description: "Lower peak frequency (~52 kHz) than M. microtis. Gleaning insectivore. Low-intensity multiharmonic FM. Documented from Costa Rica.",
    echolocates: true,
};

pub const LOPHOSTOMA_SILVICOLUM: BatSpecies = BatSpecies {
    id: "lophostoma_silvicolum",
    name: "White-throated Round-eared Bat",
    scientific_name: "Lophostoma silvicolum",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 100_000.0,
    description: "Gleaning insectivore. Modifies termite nests to create roosts. Peak ~70 kHz. Low-intensity multiharmonic FM. Difficult to detect beyond a few meters.",
    echolocates: true,
};

pub const MIMON_CRENULATUM: BatSpecies = BatSpecies {
    id: "mimon_crenulatum",
    name: "Striped Hairy-nosed Bat",
    scientific_name: "Gardnerycteris crenulatum",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 100_000.0,
    description: "Gleaning insectivore. Peak ~75 kHz. Low-intensity multiharmonic FM. Now placed in genus Gardnerycteris by some authorities.",
    echolocates: true,
};

pub const TONATIA_SAUROPHILA: BatSpecies = BatSpecies {
    id: "tonatia_saurophila",
    name: "Stripe-headed Round-eared Bat",
    scientific_name: "Tonatia saurophila",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 90_000.0,
    description: "Gleaning insectivore/carnivore. Peak ~65 kHz. Low-intensity multiharmonic FM. Forest interior species.",
    echolocates: true,
};

pub const MACROPHYLLUM_MACROPHYLLUM: BatSpecies = BatSpecies {
    id: "macrophyllum_macrophyllum",
    name: "Long-legged Bat",
    scientific_name: "Macrophyllum macrophyllum",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 110_000.0,
    description: "Unusual phyllostomid: trawls insects from water surfaces. Louder than most phyllostomids (~101 dB SPL). Adjusts intensity by habitat openness. Peak ~85 kHz.",
    echolocates: true,
};

pub const LAMPRONYCTERIS_BRACHYOTIS: BatSpecies = BatSpecies {
    id: "lampronycteris_brachyotis",
    name: "Yellow-throated Big-eared Bat",
    scientific_name: "Lampronycteris brachyotis",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 100_000.0,
    description: "Rare gleaning insectivore. Peak ~75 kHz. Low-intensity multiharmonic FM. Poorly documented acoustically.",
    echolocates: true,
};

pub const GLYPHONYCTERIS_SYLVESTRIS: BatSpecies = BatSpecies {
    id: "glyphonycteris_sylvestris",
    name: "Tricolored Big-eared Bat",
    scientific_name: "Glyphonycteris sylvestris",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 110_000.0,
    description: "Rare gleaning insectivore. Peak ~85 kHz. Very short broadband FM calls (0.3\u{2013}1 ms). Ultra-low intensity.",
    echolocates: true,
};

pub const TRINYCTERIS_NICEFORI: BatSpecies = BatSpecies {
    id: "trinycteris_nicefori",
    name: "Niceforo's Big-eared Bat",
    scientific_name: "Trinycteris nicefori",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 100_000.0,
    description: "Low-intensity gleaner. Peak ~80 kHz. Multiharmonic FM. Forest understory specialist.",
    echolocates: true,
};

// ── Phyllostomidae: Glossophaginae (nectar bats) ───────────────────────────

pub const GLOSSOPHAGA_SORICINA: BatSpecies = BatSpecies {
    id: "glossophaga_soricina",
    name: "Pallas's Long-tongued Bat",
    scientific_name: "Glossophaga soricina",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 140_000.0,
    description: "Nectarivore that uses echolocation to find flowers (some have evolved acoustic reflectors). Peak ~80 kHz. Multiharmonic FM. Low intensity.",
    echolocates: true,
};

pub const GLOSSOPHAGA_COMMISSARISI: BatSpecies = BatSpecies {
    id: "glossophaga_commissarisi",
    name: "Commissaris's Long-tongued Bat",
    scientific_name: "Glossophaga commissarisi",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 130_000.0,
    description: "Similar call structure to G. soricina but slightly lower peak (~75 kHz). Nectarivore. Low-intensity multiharmonic FM.",
    echolocates: true,
};

pub const ANOURA_GEOFFROYI: BatSpecies = BatSpecies {
    id: "anoura_geoffroyi",
    name: "Geoffroy's Tailless Bat",
    scientific_name: "Anoura geoffroyi",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 120_000.0,
    description: "High-altitude nectarivore. Peak ~70 kHz. Low-intensity multiharmonic FM. Found in cloud forests and highlands.",
    echolocates: true,
};

pub const HYLONYCTERIS_UNDERWOODI: BatSpecies = BatSpecies {
    id: "hylonycteris_underwoodi",
    name: "Underwood's Long-tongued Bat",
    scientific_name: "Hylonycteris underwoodi",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 130_000.0,
    description: "Very small nectarivore. High frequency calls (~90 kHz peak). Low-intensity FM. Found in montane forests.",
    echolocates: true,
};

// ── Phyllostomidae: Carolliinae (short-tailed bats) ────────────────────────

pub const CAROLLIA_PERSPICILLATA: BatSpecies = BatSpecies {
    id: "carollia_perspicillata",
    name: "Seba's Short-tailed Bat",
    scientific_name: "Carollia perspicillata",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 100_000.0,
    description: "One of the most abundant Neotropical bats. Peak ~71 kHz. Frugivore that also takes insects. Low-intensity whispering calls.",
    echolocates: true,
};

pub const CAROLLIA_CASTANEA: BatSpecies = BatSpecies {
    id: "carollia_castanea",
    name: "Chestnut Short-tailed Bat",
    scientific_name: "Carollia castanea",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 110_000.0,
    description: "Higher peak (~78 kHz) than C. perspicillata, consistent with smaller body. Frugivore. Low-intensity FM. Documented from Costa Rica.",
    echolocates: true,
};

pub const CAROLLIA_BREVICAUDA: BatSpecies = BatSpecies {
    id: "carollia_brevicauda",
    name: "Silky Short-tailed Bat",
    scientific_name: "Carollia brevicauda",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 100_000.0,
    description: "Intermediate in size and frequency (~73 kHz peak) between C. perspicillata and C. castanea. Frugivore. Low-intensity FM.",
    echolocates: true,
};

// ── Phyllostomidae: Desmodontinae (vampire bats) ──────────────────────────

pub const DESMODUS_ROTUNDUS: BatSpecies = BatSpecies {
    id: "desmodus_rotundus",
    name: "Common Vampire Bat",
    scientific_name: "Desmodus rotundus",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 85_000.0,
    description: "Relatively long calls for a phyllostomid (~5.5 ms). Peak ~55 kHz. Obligate blood-feeder. Emits single pulses or groups of 2\u{2013}3.",
    echolocates: true,
};

// ── Phyllostomidae: Stenodermatinae (fruit bats) ──────────────────────────

pub const ARTIBEUS_JAMAICENSIS: BatSpecies = BatSpecies {
    id: "artibeus_jamaicensis",
    name: "Jamaican Fruit Bat",
    scientific_name: "Artibeus jamaicensis",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 90_000.0,
    description: "Very common frugivore. Peak ~56 kHz. Variable intensity (mean ~96 dB SPL); not always a quiet \"whisperer\". Important seed disperser.",
    echolocates: true,
};

pub const ARTIBEUS_LITURATUS: BatSpecies = BatSpecies {
    id: "artibeus_lituratus",
    name: "Great Fruit-eating Bat",
    scientific_name: "Artibeus lituratus",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 85_000.0,
    description: "Large frugivore. Lower peak (~52 kHz) than A. jamaicensis, consistent with larger body. Prominent facial stripes. Important pollinator.",
    echolocates: true,
};

pub const STURNIRA_LILIUM: BatSpecies = BatSpecies {
    id: "sturnira_lilium",
    name: "Little Yellow-shouldered Bat",
    scientific_name: "Sturnira lilium",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 76_000.0,
    description: "Frugivore. Peak ~66.5 kHz (well documented). Low-intensity multiharmonic FM. Common in forest and edge habitats.",
    echolocates: true,
};

pub const URODERMA_BILOBATUM: BatSpecies = BatSpecies {
    id: "uroderma_bilobatum",
    name: "Tent-making Bat",
    scientific_name: "Uroderma bilobatum",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 95_000.0,
    description: "Tent-roosting frugivore. Modifies large leaves into tents. Peak ~70 kHz. Low-intensity nasal FM. Found in lowland forests.",
    echolocates: true,
};

pub const ECTOPHYLLA_ALBA: BatSpecies = BatSpecies {
    id: "ectophylla_alba",
    name: "Honduran White Bat",
    scientific_name: "Ectophylla alba",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 95_000.0,
    description: "Tiny, iconic white tent-roosting bat. Peak ~75 kHz. Low-intensity FM. Endemic to Central America (Honduras to Panama). Frugivore specializing on one fig species.",
    echolocates: true,
};

pub const CENTURIO_SENEX: BatSpecies = BatSpecies {
    id: "centurio_senex",
    name: "Wrinkle-faced Bat",
    scientific_name: "Centurio senex",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 90_000.0,
    description: "Bizarre wrinkled face with skin fold that can cover it like a mask. Peak ~65 kHz. Relatively long calls for a stenodermatine (1\u{2013}3 ms). Frugivore.",
    echolocates: true,
};

pub const DERMANURA_PHAEOTIS: BatSpecies = BatSpecies {
    id: "dermanura_phaeotis",
    name: "Pygmy Fruit-eating Bat",
    scientific_name: "Dermanura phaeotis",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 100_000.0,
    description: "Small frugivore (formerly Artibeus phaeotis). Peak ~75 kHz. Low-intensity multiharmonic FM. Common in lowland forests.",
    echolocates: true,
};

pub const MESOPHYLLA_MACCONNELLI: BatSpecies = BatSpecies {
    id: "mesophylla_macconnelli",
    name: "MacConnell's Bat",
    scientific_name: "Mesophylla macconnelli",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 70_000.0,
    freq_hi_hz: 118_000.0,
    description: "Very small (5\u{2013}7 g) with the highest peak frequency recorded among phyllostomids (~100\u{2013}118 kHz). Ultra-low intensity. Tent-roosting frugivore.",
    echolocates: true,
};

// ── Molossidae ─────────────────────────────────────────────────────────────

pub const MOLOSSUS_MOLOSSUS: BatSpecies = BatSpecies {
    id: "molossus_molossus",
    name: "Velvety Free-tailed Bat",
    scientific_name: "Molossus molossus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 33_000.0,
    freq_hi_hz: 50_000.0,
    description: "Alternates between ~34.5 and ~39.6 kHz when foraging. Narrowband QCF. Common in buildings and urban areas. Open-space aerial hawker.",
    echolocates: true,
};

pub const MOLOSSUS_SINALOAE: BatSpecies = BatSpecies {
    id: "molossus_sinaloae",
    name: "Sinaloan Mastiff Bat",
    scientific_name: "Molossus sinaloae",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 31_000.0,
    freq_hi_hz: 57_000.0,
    description: "QCF at ~34 kHz in natural habitat; shifts up ~6 kHz in urban noise (Lombard effect). Open-space forager. Larger than M. molossus.",
    echolocates: true,
};

pub const MOLOSSUS_RUFUS: BatSpecies = BatSpecies {
    id: "molossus_rufus",
    name: "Black Mastiff Bat",
    scientific_name: "Molossus rufus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 24_000.0,
    freq_hi_hz: 28_000.0,
    description: "Large molossid with low frequency (~25\u{2013}26 kHz). Open-space forager. Formerly M. ater. Roosts in buildings and hollow trees.",
    echolocates: true,
};

pub const MOLOSSUS_BONDAE: BatSpecies = BatSpecies {
    id: "molossus_bondae",
    name: "Bonda Mastiff Bat",
    scientific_name: "Molossus bondae",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 42_000.0,
    description: "Similar to M. molossus but slightly lower frequency (~33 kHz). Open-space forager. Roosts in buildings.",
    echolocates: true,
};

pub const MOLOSSUS_COIBENSIS: BatSpecies = BatSpecies {
    id: "molossus_coibensis",
    name: "Coiban Mastiff Bat",
    scientific_name: "Molossus coibensis",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 42_000.0,
    description: "Similar to M. molossus (~35 kHz). Originally described from Coiba Island, Panama. Open-space forager.",
    echolocates: true,
};

pub const EUMOPS_AURIPENDULUS: BatSpecies = BatSpecies {
    id: "eumops_auripendulus",
    name: "Black Bonneted Bat",
    scientific_name: "Eumops auripendulus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 18_000.0,
    freq_hi_hz: 36_000.0,
    description: "Large molossid. Alternating QCF calls at ~23\u{2013}26 kHz. High, fast flight. Long-duration narrowband calls.",
    echolocates: true,
};

pub const EUMOPS_GLAUCINUS: BatSpecies = BatSpecies {
    id: "eumops_glaucinus",
    name: "Wagner's Bonneted Bat",
    scientific_name: "Eumops glaucinus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 19_000.0,
    freq_hi_hz: 29_000.0,
    description: "Among the lowest-frequency calling molossids (~22\u{2013}25 kHz). Large bat with long, narrow wings. High-altitude open-space forager.",
    echolocates: true,
};

pub const CYNOMOPS_GREENHALLI: BatSpecies = BatSpecies {
    id: "cynomops_greenhalli",
    name: "Greenhall's Dog-faced Bat",
    scientific_name: "Cynomops greenhalli",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 17_000.0,
    freq_hi_hz: 29_000.0,
    description: "Low frequency (~22 kHz) open-space forager. Flat-faced with forward-pointing nostrils. Roosts in buildings and hollow trees.",
    echolocates: true,
};

pub const TADARIDA_BRASILIENSIS: BatSpecies = BatSpecies {
    id: "tadarida_brasiliensis",
    name: "Mexican Free-tailed Bat",
    scientific_name: "Tadarida brasiliensis",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 75_000.0,
    description: "Extremely flexible acoustics: narrow QCF 49\u{2013}70 kHz in open space, drops to 25\u{2013}40 kHz near objects, broadband FM during approach. Forms massive colonies.",
    echolocates: true,
};

pub const PROMOPS_CENTRALIS: BatSpecies = BatSpecies {
    id: "promops_centralis",
    name: "Big-crested Mastiff Bat",
    scientific_name: "Promops centralis",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 35_000.0,
    description: "Distinctive upward-modulated calls (unusual for molossids). Alternating pulse pairs at ~30 and ~35 kHz. Most energy in 1st harmonic. Easily recognizable.",
    echolocates: true,
};

pub const NYCTINOMOPS_LATICAUDATUS: BatSpecies = BatSpecies {
    id: "nyctinomops_laticaudatus",
    name: "Broad-eared Free-tailed Bat",
    scientific_name: "Nyctinomops laticaudatus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 24_000.0,
    freq_hi_hz: 32_000.0,
    description: "Three-frequency alternation pattern (~26.7, 28.7, 32.4 kHz). Downward-modulated QCF. Open-space forager.",
    echolocates: true,
};

// ── Vespertilionidae ───────────────────────────────────────────────────────

pub const MYOTIS_NIGRICANS: BatSpecies = BatSpecies {
    id: "myotis_nigricans",
    name: "Black Myotis",
    scientific_name: "Myotis nigricans",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 80_000.0,
    description: "Highly plastic call structure. Narrowband ~7 ms in open space; adds broadband FM in clutter. Convergent with European pipistrelles. Peak ~50 kHz.",
    echolocates: true,
};

pub const MYOTIS_KEAYSI: BatSpecies = BatSpecies {
    id: "myotis_keaysi",
    name: "Hairy-legged Myotis",
    scientific_name: "Myotis keaysi",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 100_000.0,
    description: "High variable repetition rates (15\u{2013}20 calls/s). Short, high-intensity calls (~2.5 ms). Peak ~55 kHz. Recorded from colonies in Costa Rica.",
    echolocates: true,
};

pub const MYOTIS_RIPARIUS: BatSpecies = BatSpecies {
    id: "myotis_riparius",
    name: "Riparian Myotis",
    scientific_name: "Myotis riparius",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 120_000.0,
    description: "Steep broadband FM sweep from ~120 kHz to ~50 kHz. Short calls (~2 ms). No harmonics observed. Recorded in Costa Rica.",
    echolocates: true,
};

pub const MYOTIS_ELEGANS: BatSpecies = BatSpecies {
    id: "myotis_elegans",
    name: "Elegant Myotis",
    scientific_name: "Myotis elegans",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 100_000.0,
    description: "High-frequency calls typical of small Myotis. Peak ~55 kHz. Difficult to distinguish from M. nigricans acoustically.",
    echolocates: true,
};

pub const EPTESICUS_BRASILIENSIS: BatSpecies = BatSpecies {
    id: "eptesicus_brasiliensis",
    name: "Brazilian Brown Bat",
    scientific_name: "Eptesicus brasiliensis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 70_000.0,
    description: "Peak ~54\u{2013}60 kHz. Source level ~101\u{2013}106 dB SPL. Frequency decreases with temperature. FM-QCF structure.",
    echolocates: true,
};

pub const EPTESICUS_FURINALIS: BatSpecies = BatSpecies {
    id: "eptesicus_furinalis",
    name: "Argentine Brown Bat",
    scientific_name: "Eptesicus furinalis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 45_000.0,
    description: "Lower frequency than E. brasiliensis (~43 kHz peak). More FM in cluttered habitats. FM-QCF structure.",
    echolocates: true,
};

pub const EPTESICUS_FUSCUS: BatSpecies = BatSpecies {
    id: "eptesicus_fuscus",
    name: "Big Brown Bat",
    scientific_name: "Eptesicus fuscus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 50_000.0,
    description: "One of the best-studied bat species globally. Large vespertilionid. Peak ~30 kHz. FM-QCF. Open-space forager. Common in buildings.",
    echolocates: true,
};

pub const LASIURUS_BLOSSEVILLII: BatSpecies = BatSpecies {
    id: "lasiurus_blossevillii",
    name: "Desert Red Bat",
    scientific_name: "Lasiurus blossevillii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 23_000.0,
    freq_hi_hz: 52_000.0,
    description: "Open-air forager. Maximum energy ~42 kHz. FM-QCF. Migratory. Roosts solitarily in foliage. Distinctive reddish fur.",
    echolocates: true,
};

pub const LASIURUS_EGA: BatSpecies = BatSpecies {
    id: "lasiurus_ega",
    name: "Southern Yellow Bat",
    scientific_name: "Lasiurus ega",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 50_000.0,
    description: "Lower peak (~35 kHz) than L. blossevillii. FM-QCF. Open-air forager. Roosts in palm fronds.",
    echolocates: true,
};

pub const RHOGEESSA_TUMIDA: BatSpecies = BatSpecies {
    id: "rhogeessa_tumida",
    name: "Black-winged Little Yellow Bat",
    scientific_name: "Rhogeessa tumida",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 43_000.0,
    freq_hi_hz: 55_000.0,
    description: "Small vespertilionid. Broadband steep FM with small QCF termination. Peak ~48 kHz. Bandwidth >15 kHz.",
    echolocates: true,
};

pub const BAUERUS_DUBIAQUERCUS: BatSpecies = BatSpecies {
    id: "bauerus_dubiaquercus",
    name: "Van Gelder's Bat",
    scientific_name: "Bauerus dubiaquercus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 55_000.0,
    description: "Very quiet calls. Related to long-eared bats (Plecotus-like). Gleaning insectivore. Peak ~35 kHz. Rare, poorly known.",
    echolocates: true,
};

// ── Thyropteridae ──────────────────────────────────────────────────────────

pub const THYROPTERA_TRICOLOR: BatSpecies = BatSpecies {
    id: "thyroptera_tricolor",
    name: "Spix's Disk-winged Bat",
    scientific_name: "Thyroptera tricolor",
    family: "Thyropteridae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 70_000.0,
    description: "Suction-cup disks on wrists and ankles for roosting in rolled Heliconia leaves. Extremely low intensity; barely detectable at <1 m. Distinctive social calls for roost-finding.",
    echolocates: true,
};

// ── Natalidae ──────────────────────────────────────────────────────────────

pub const NATALUS_MEXICANUS: BatSpecies = BatSpecies {
    id: "natalus_mexicanus",
    name: "Mexican Funnel-eared Bat",
    scientific_name: "Natalus mexicanus",
    family: "Natalidae",
    call_type: "FM",
    freq_lo_hz: 85_000.0,
    freq_hi_hz: 170_000.0,
    description: "Among the highest frequency echolocating bats. Peak ~100\u{2013}130 kHz (2nd harmonic). Very low intensity; barely detectable beyond 50 cm. Cave-roosting.",
    echolocates: true,
};

// ══════════════════════════════════════════════════════════════════════════════
// Japan species
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Fukui et al. (2004) Zool Sci: Acoustic ID of eight Hokkaido species
// - Funakoshi & Uchida (1978): R. ferrumequinum nippon CF frequency
// - Hiryu et al. (2006): R. pumilus geographic variation on Okinawa
// - Fujioka et al. (2014): CNN bat species ID system for 30 Japanese species
// - IUCN Red List; Ministry of the Environment (Japan) Red Data Book

// ── Pteropodidae ─────────────────────────────────────────────────────────────

pub const PTEROPUS_DASYMALLUS: BatSpecies = BatSpecies {
    id: "pteropus_dasymallus",
    name: "Ryukyu Flying Fox",
    scientific_name: "Pteropus dasymallus",
    family: "Pteropodidae",
    call_type: "none",
    freq_lo_hz: 0.0,
    freq_hi_hz: 0.0,
    description: "Large fruit bat of the Ryukyu Islands (4 subspecies in Japan). Does not echolocate. Feeds on fruit, flowers, and leaves. Endangered due to habitat loss and hunting.",
    echolocates: false,
};

// ── Rhinolophidae ────────────────────────────────────────────────────────────

pub const RHINOLOPHUS_FERRUMEQUINUM_NIPPON: BatSpecies = BatSpecies {
    id: "rhinolophus_ferrumequinum_nippon",
    name: "Japanese Greater Horseshoe Bat",
    scientific_name: "Rhinolophus ferrumequinum nippon",
    family: "Rhinolophidae",
    call_type: "CF-FM",
    freq_lo_hz: 58_000.0,
    freq_hi_hz: 72_000.0,
    description: "CF-FM calls with CF2 at ~65\u{2013}69 kHz. FM/CF/FM structure. Widespread from Hokkaido to Kyushu. Roosts in caves, mines, and tunnels.",
    echolocates: true,
};

pub const RHINOLOPHUS_CORNUTUS: BatSpecies = BatSpecies {
    id: "rhinolophus_cornutus",
    name: "Japanese Lesser Horseshoe Bat",
    scientific_name: "Rhinolophus cornutus",
    family: "Rhinolophidae",
    call_type: "CF-FM",
    freq_lo_hz: 100_000.0,
    freq_hi_hz: 120_000.0,
    description: "CF-FM calls with peak CF ~103\u{2013}111 kHz, increasing from north to south. Endemic to Japan (Honshu, Shikoku, Kyushu). Roosts in caves and buildings.",
    echolocates: true,
};

pub const RHINOLOPHUS_PUMILUS: BatSpecies = BatSpecies {
    id: "rhinolophus_pumilus",
    name: "Okinawa Least Horseshoe Bat",
    scientific_name: "Rhinolophus pumilus",
    family: "Rhinolophidae",
    call_type: "CF-FM",
    freq_lo_hz: 80_000.0,
    freq_hi_hz: 92_000.0,
    description: "CF-FM calls with CF ~80\u{2013}90 kHz. Shows 5\u{2013}8 kHz dialect difference between northern and southern Okinawa populations. Endemic to the central and southern Ryukyus.",
    echolocates: true,
};

pub const RHINOLOPHUS_PERDITUS: BatSpecies = BatSpecies {
    id: "rhinolophus_perditus",
    name: "Yaeyama Horseshoe Bat",
    scientific_name: "Rhinolophus perditus",
    family: "Rhinolophidae",
    call_type: "CF-FM",
    freq_lo_hz: 90_000.0,
    freq_hi_hz: 100_000.0,
    description: "CF-FM calls with peak CF ~92\u{2013}98 kHz (92\u{2013}93 on Iriomote, 96\u{2013}98 on Ishigaki). Endemic to the Yaeyama Islands. Forest-dwelling, cave-roosting.",
    echolocates: true,
};

pub const RHINOLOPHUS_IMAIZUMII: BatSpecies = BatSpecies {
    id: "rhinolophus_imaizumii",
    name: "Imaizumi's Horseshoe Bat",
    scientific_name: "Rhinolophus imaizumii",
    family: "Rhinolophidae",
    call_type: "CF-FM",
    freq_lo_hz: 94_000.0,
    freq_hi_hz: 108_000.0,
    description: "CF-FM calls at frequency intermediate between R. cornutus and R. pumilus. Temperate forests on Honshu and Shikoku. Taxonomic status debated (sometimes synonymised with R. pusillus).",
    echolocates: true,
};

// ── Hipposideridae ───────────────────────────────────────────────────────────

pub const HIPPOSIDEROS_TURPIS: BatSpecies = BatSpecies {
    id: "hipposideros_turpis",
    name: "Temminck's Leaf-nosed Bat",
    scientific_name: "Hipposideros turpis",
    family: "Hipposideridae",
    call_type: "CF",
    freq_lo_hz: 65_000.0,
    freq_hi_hz: 80_000.0,
    description: "CF calls typical of hipposiderids. Southern Ryukyu Islands (Ishigaki, Iriomote, Miyako). Roosts in limestone caves. Feeds mainly on beetles.",
    echolocates: true,
};

// ── Emballonuridae ───────────────────────────────────────────────────────────

pub const TAPHOZOUS_MELANOPOGON: BatSpecies = BatSpecies {
    id: "taphozous_melanopogon",
    name: "Black-bearded Tomb Bat",
    scientific_name: "Taphozous melanopogon",
    family: "Emballonuridae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 40_000.0,
    description: "Low-intensity FM calls, dominant frequency ~29\u{2013}33 kHz with four harmonics and most energy in the second. Ryukyu Islands. Roosts on rock walls and in caves.",
    echolocates: true,
};

// ── Miniopteridae ────────────────────────────────────────────────────────────

pub const MINIOPTERUS_FULIGINOSUS: BatSpecies = BatSpecies {
    id: "miniopterus_fuliginosus",
    name: "Eastern Bent-wing Bat",
    scientific_name: "Miniopterus fuliginosus",
    family: "Miniopteridae",
    call_type: "FM",
    freq_lo_hz: 44_000.0,
    freq_hi_hz: 100_000.0,
    description: "FM downward sweeps, terminal frequency ~44\u{2013}50 kHz. Fast agile flier. Cave-roosting, forms large maternity colonies. Widespread from Honshu to the Ryukyus.",
    echolocates: true,
};

pub const MINIOPTERUS_FUSCUS: BatSpecies = BatSpecies {
    id: "miniopterus_fuscus",
    name: "Ryukyu Bent-wing Bat",
    scientific_name: "Miniopterus fuscus",
    family: "Miniopteridae",
    call_type: "FM",
    freq_lo_hz: 48_000.0,
    freq_hi_hz: 105_000.0,
    description: "FM calls slightly higher frequency than M. fuliginosus due to smaller body size. Ryukyu Islands and southern Kyushu. Cave-roosting.",
    echolocates: true,
};

// ── Molossidae ───────────────────────────────────────────────────────────────

pub const TADARIDA_INSIGNIS: BatSpecies = BatSpecies {
    id: "tadarida_insignis",
    name: "East Asian Free-tailed Bat",
    scientific_name: "Tadarida insignis",
    family: "Molossidae",
    call_type: "FM-QCF",
    freq_lo_hz: 15_000.0,
    freq_hi_hz: 30_000.0,
    description: "Low-frequency FM-QCF calls ~18\u{2013}25 kHz. Fast, high-flying open-air forager. Western Japan. Roosts in rock crevices and buildings.",
    echolocates: true,
};

pub const TADARIDA_LATOUCHEI: BatSpecies = BatSpecies {
    id: "tadarida_latouchei",
    name: "La Touche's Free-tailed Bat",
    scientific_name: "Tadarida latouchei",
    family: "Molossidae",
    call_type: "FM-QCF",
    freq_lo_hz: 15_000.0,
    freq_hi_hz: 25_000.0,
    description: "Echolocation ~20 kHz. High-altitude flier, very difficult to capture. Known in Japan from a single specimen on Amami-Oshima (1985). IUCN Data Deficient.",
    echolocates: true,
};

// ── Vespertilionidae: Myotis ─────────────────────────────────────────────────

pub const MYOTIS_MACRODACTYLUS: BatSpecies = BatSpecies {
    id: "myotis_macrodactylus",
    name: "Large-footed Bat",
    scientific_name: "Myotis macrodactylus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 90_000.0,
    description: "Steep FM sweeps ~90\u{2013}40 kHz, peak ~50 kHz. Trawling insectivore, hunts over water using large feet to gaff prey. Rivers and streams throughout Japan.",
    echolocates: true,
};

pub const MYOTIS_IKONNIKOVI: BatSpecies = BatSpecies {
    id: "myotis_ikonnikovi",
    name: "Ikonnikov's Bat",
    scientific_name: "Myotis ikonnikovi",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 37_000.0,
    freq_hi_hz: 120_000.0,
    description: "Steep FM sweeps, peak ~50.6 kHz, start ~90 kHz, end ~43 kHz, duration ~2 ms. Small forest bat. Hokkaido and northern Honshu.",
    echolocates: true,
};

pub const MYOTIS_BOMBINUS: BatSpecies = BatSpecies {
    id: "myotis_bombinus",
    name: "Far Eastern Natterer's Bat",
    scientific_name: "Myotis bombinus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 150_000.0,
    description: "Very broadband FM calls sweeping from ~150 kHz down to ~20 kHz. Gleaning insectivore, hawks near vegetation. Forests of Kyushu and other regions.",
    echolocates: true,
};

pub const MYOTIS_PRUINOSUS: BatSpecies = BatSpecies {
    id: "myotis_pruinosus",
    name: "Frosted Myotis",
    scientific_name: "Myotis pruinosus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 100_000.0,
    description: "FM sweeps typical of Myotis. Endemic to Japan (Honshu, Shikoku, Kyushu). Forest-dwelling, roosts in tree hollows and buildings. Named for frosted fur.",
    echolocates: true,
};

pub const MYOTIS_YANBARENSIS: BatSpecies = BatSpecies {
    id: "myotis_yanbarensis",
    name: "Yanbaru Whiskered Bat",
    scientific_name: "Myotis yanbarensis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 100_000.0,
    description: "FM calls above 40 kHz. Endemic to northern Okinawa (Yanbaru forest). Described in 1997. One of the rarest bats in Japan.",
    echolocates: true,
};

pub const MYOTIS_FRATER: BatSpecies = BatSpecies {
    id: "myotis_frater",
    name: "Fraternal Bat",
    scientific_name: "Myotis frater",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 110_000.0,
    description: "Brief FM pulses ~110\u{2013}50 kHz, duration ~3.5 ms. Edge-space forager. Honshu and Kyushu in forested areas near cliffs and caves.",
    echolocates: true,
};

pub const MYOTIS_FORMOSUS: BatSpecies = BatSpecies {
    id: "myotis_formosus",
    name: "Hodgson's Bat",
    scientific_name: "Myotis formosus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 95_000.0,
    description: "Steep downward FM calls. Distinctive orange-brown coloring. Tsushima Island and western Japan. Forest-dwelling insectivore.",
    echolocates: true,
};

pub const MYOTIS_HOSONOI: BatSpecies = BatSpecies {
    id: "myotis_hosonoi",
    name: "Hosono's Myotis",
    scientific_name: "Myotis hosonoi",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 95_000.0,
    description: "FM calls typical of Myotis. Endemic to Japan (Honshu). Cave-dwelling. Poorly studied.",
    echolocates: true,
};

pub const MYOTIS_GRACILIS: BatSpecies = BatSpecies {
    id: "myotis_gracilis",
    name: "Gracile Myotis",
    scientific_name: "Myotis gracilis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 100_000.0,
    description: "FM calls typical of small Myotis. Endemic to Japan (Honshu). Forest-dwelling. Limited published acoustic data.",
    echolocates: true,
};

pub const MYOTIS_OZENSIS: BatSpecies = BatSpecies {
    id: "myotis_ozensis",
    name: "Ozensis Myotis",
    scientific_name: "Myotis ozensis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 95_000.0,
    description: "FM calls typical of Myotis. Endemic to Japan (central Honshu mountains). Cave-dwelling. Limited distribution.",
    echolocates: true,
};

// ── Vespertilionidae: Pipistrellus ───────────────────────────────────────────

pub const PIPISTRELLUS_ABRAMUS: BatSpecies = BatSpecies {
    id: "pipistrellus_abramus",
    name: "Japanese Pipistrelle",
    scientific_name: "Pipistrellus abramus",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 95_000.0,
    description: "FM sweeps ~80\u{2013}95 kHz to terminal ~40 kHz, peak ~52 kHz. Japan's most common urban bat. Roosts in buildings and under bridges. Forages around streetlights.",
    echolocates: true,
};

pub const PIPISTRELLUS_ENDOI: BatSpecies = BatSpecies {
    id: "pipistrellus_endoi",
    name: "Endo's Pipistrelle",
    scientific_name: "Pipistrellus endoi",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 38_000.0,
    freq_hi_hz: 90_000.0,
    description: "FM-QCF calls similar to P. abramus but slightly lower frequency. Endemic to Japan (Honshu). Temperate forests at 100\u{2013}1500 m. IUCN Near Threatened.",
    echolocates: true,
};

pub const PIPISTRELLUS_STURDEEI: BatSpecies = BatSpecies {
    id: "pipistrellus_sturdeei",
    name: "Sturdee's Pipistrelle",
    scientific_name: "Pipistrellus sturdeei",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 38_000.0,
    freq_hi_hz: 90_000.0,
    description: "Presumed FM-QCF calls. Known only from a specimen on the Bonin Islands (Ogasawara). Not seen since 1915\u{2014}possibly extinct.",
    echolocates: true,
};

// ── Vespertilionidae: Vespertilio ────────────────────────────────────────────

pub const VESPERTILIO_SINENSIS: BatSpecies = BatSpecies {
    id: "vespertilio_sinensis",
    name: "Asian Parti-coloured Bat",
    scientific_name: "Vespertilio sinensis",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 46_000.0,
    description: "FM-QCF calls, peak ~24 kHz, max ~46 kHz, min ~22\u{2013}23 kHz. Steep FM into shallow QCF tail. Migratory. Hokkaido to Kyushu. Roosts in tree hollows and buildings.",
    echolocates: true,
};

// ── Vespertilionidae: Eptesicus ──────────────────────────────────────────────

pub const EPTESICUS_NILSSONII_JP: BatSpecies = BatSpecies {
    id: "eptesicus_nilssonii_jp",
    name: "Northern Bat",
    scientific_name: "Eptesicus nilssonii",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 55_000.0,
    description: "FM-QCF calls, peak ~30.5 kHz, duration ~6 ms, bandwidth ~32 kHz. Hokkaido and northern Honshu. The most northerly-ranging bat in the world.",
    echolocates: true,
};

pub const EPTESICUS_JAPONENSIS: BatSpecies = BatSpecies {
    id: "eptesicus_japonensis",
    name: "Japanese Short-tailed Bat",
    scientific_name: "Eptesicus japonensis",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 55_000.0,
    description: "FM-QCF calls similar to E. nilssonii. Endemic to Japan (Honshu, Shikoku, Kyushu). Forest-dwelling, roosts in tree hollows and buildings.",
    echolocates: true,
};

// ── Vespertilionidae: Nyctalus ───────────────────────────────────────────────

pub const NYCTALUS_AVIATOR: BatSpecies = BatSpecies {
    id: "nyctalus_aviator",
    name: "Japanese Noctule",
    scientific_name: "Nyctalus aviator",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 17_000.0,
    freq_hi_hz: 54_000.0,
    description: "FM-QCF calls, peak ~21 kHz, duration ~12 ms. Japan's largest open-space insectivorous bat. Forages up to 300 m altitude. Occasionally preys on migrating birds. Roosts in tree cavities.",
    echolocates: true,
};

// ── Vespertilionidae: Plecotus ───────────────────────────────────────────────

pub const PLECOTUS_SACRIMONTIS: BatSpecies = BatSpecies {
    id: "plecotus_sacrimontis",
    name: "Japanese Long-eared Bat",
    scientific_name: "Plecotus sacrimontis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 23_000.0,
    freq_hi_hz: 46_000.0,
    description: "Low-intensity FM calls, start ~46 kHz, peak ~41 kHz, end ~23 kHz. Gleaning forager specialising in fluttering moths. Endemic to Japan (Honshu, Hokkaido, Shikoku, Kyushu).",
    echolocates: true,
};

// ── Vespertilionidae: Barbastella ────────────────────────────────────────────

pub const BARBASTELLA_LEUCOMELAS: BatSpecies = BatSpecies {
    id: "barbastella_leucomelas",
    name: "Asian Barbastelle",
    scientific_name: "Barbastella leucomelas",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 47_000.0,
    description: "FM calls in two alternating types: type A at 32\u{2013}37 kHz, type B at 38\u{2013}45 kHz. Similar to European B. barbastellus. Rare in Japan (Honshu). Forest-dwelling, very elusive.",
    echolocates: true,
};

// ── Vespertilionidae: Murina (tube-nosed bats) ──────────────────────────────

pub const MURINA_HILGENDORFI: BatSpecies = BatSpecies {
    id: "murina_hilgendorfi",
    name: "Hilgendorf's Tube-nosed Bat",
    scientific_name: "Murina hilgendorfi",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 165_000.0,
    description: "Ultra-broadband FM sweeps ~145\u{2013}165 kHz down to ~45\u{2013}55 kHz. Forest gleaner. Throughout Japan (Hokkaido to Kyushu). Roosts in tree hollows and dead curled leaves.",
    echolocates: true,
};

pub const MURINA_USSURIENSIS: BatSpecies = BatSpecies {
    id: "murina_ussuriensis",
    name: "Ussuri Tube-nosed Bat",
    scientific_name: "Murina ussuriensis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 165_000.0,
    description: "Ultra-broadband FM sweeps similar to M. hilgendorfi. Tiny bat (4\u{2013}8 g). Remarkably hibernates under snow. Roosts in curled dead leaves. Hokkaido and Honshu.",
    echolocates: true,
};

pub const MURINA_SILVATICA: BatSpecies = BatSpecies {
    id: "murina_silvatica",
    name: "Ognev's Tube-nosed Bat",
    scientific_name: "Murina silvatica",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 160_000.0,
    description: "Broadband FM calls typical of Murina. Distribution spans ~2000 km north\u{2013}south across the Japanese archipelago. Forest bat roosting in dead curled leaves.",
    echolocates: true,
};

pub const MURINA_TENEBROSA: BatSpecies = BatSpecies {
    id: "murina_tenebrosa",
    name: "Gloomy Tube-nosed Bat",
    scientific_name: "Murina tenebrosa",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 160_000.0,
    description: "FM calls presumed similar to other Murina. Known only from a single holotype on Tsushima Island (1962). Possibly extinct. Alliance for Zero Extinction species.",
    echolocates: true,
};

pub const MURINA_RYUKYUANA: BatSpecies = BatSpecies {
    id: "murina_ryukyuana",
    name: "Ryukyu Tube-nosed Bat",
    scientific_name: "Murina ryukyuana",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 160_000.0,
    description: "Broadband FM calls typical of Murina. Endemic to the Ryukyu Islands. Recently described species from Okinawa. Forest-dwelling.",
    echolocates: true,
};

// ══════════════════════════════════════════════════════════════════════════════
// North America species (USA + Canada)
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Fenton & Bell (1981): Echolocation calls of NA bats
// - O'Farrell et al. (1999): Acoustic identification of NA insectivorous bats
// - Szewczak (2004): Advanced analysis techniques for bat echolocation calls
// - Various state acoustic ID guides (e.g. Bat Call ID project, SonoBat)
// - Holroyd et al. (2014): Canadian bat species & echolocation
// - Kunz & Fenton (2003): Bat Ecology

// ── Vespertilionidae ─────────────────────────────────────────────────────────

pub const MYOTIS_LUCIFUGUS: BatSpecies = BatSpecies {
    id: "myotis_lucifugus",
    name: "Little Brown Myotis",
    scientific_name: "Myotis lucifugus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 80_000.0,
    description: "Historically North America's most common bat, now severely impacted by White-nose Syndrome. FM sweep ~80\u{2013}40 kHz, characteristic frequency ~45 kHz. Forages over water and forest edges.",
    echolocates: true,
};

pub const MYOTIS_SEPTENTRIONALIS: BatSpecies = BatSpecies {
    id: "myotis_septentrionalis",
    name: "Northern Long-eared Myotis",
    scientific_name: "Myotis septentrionalis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 115_000.0,
    description: "Steep broadband FM sweeps, short duration. Characteristic frequency ~55 kHz. Gleaning forager in forest understory. Federally endangered due to White-nose Syndrome.",
    echolocates: true,
};

pub const MYOTIS_SODALIS: BatSpecies = BatSpecies {
    id: "myotis_sodalis",
    name: "Indiana Bat",
    scientific_name: "Myotis sodalis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 80_000.0,
    description: "Calls very similar to Little Brown Myotis; characteristic frequency ~45 kHz. Hibernates in dense clusters in limestone caves. Federally endangered.",
    echolocates: true,
};

pub const MYOTIS_GRISESCENS: BatSpecies = BatSpecies {
    id: "myotis_grisescens",
    name: "Gray Myotis",
    scientific_name: "Myotis grisescens",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 70_000.0,
    description: "Largest eastern Myotis. FM sweep ~70\u{2013}35 kHz, characteristic frequency ~45 kHz. Obligate cave bat year-round. Federally endangered but populations recovering.",
    echolocates: true,
};

pub const MYOTIS_LEIBII: BatSpecies = BatSpecies {
    id: "myotis_leibii",
    name: "Eastern Small-footed Myotis",
    scientific_name: "Myotis leibii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 90_000.0,
    description: "One of North America's smallest bats (~5 g). FM sweep with characteristic frequency ~50\u{2013}55 kHz. Roosts in rock crevices and talus slopes. Appears somewhat resistant to White-nose Syndrome.",
    echolocates: true,
};

pub const MYOTIS_VOLANS: BatSpecies = BatSpecies {
    id: "myotis_volans",
    name: "Long-legged Myotis",
    scientific_name: "Myotis volans",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 80_000.0,
    description: "Western species found in coniferous forests. FM sweep with characteristic frequency ~40 kHz. Keeled calcar and furred underwing membrane diagnostic. Fast, direct flight.",
    echolocates: true,
};

pub const MYOTIS_THYSANODES: BatSpecies = BatSpecies {
    id: "myotis_thysanodes",
    name: "Fringed Myotis",
    scientific_name: "Myotis thysanodes",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 55_000.0,
    description: "Named for the fringe of stiff hairs along the tail membrane. FM sweep with characteristic frequency ~40 kHz. Roosts in caves, mines, and buildings in western mountains.",
    echolocates: true,
};

pub const MYOTIS_EVOTIS: BatSpecies = BatSpecies {
    id: "myotis_evotis",
    name: "Long-eared Myotis",
    scientific_name: "Myotis evotis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 75_000.0,
    description: "Very quiet, short FM calls typical of a gleaning forager. Characteristic frequency ~40 kHz. Large ears extend well beyond nose when laid forward. Western forests and woodlands.",
    echolocates: true,
};

pub const MYOTIS_CALIFORNICUS: BatSpecies = BatSpecies {
    id: "myotis_californicus",
    name: "California Myotis",
    scientific_name: "Myotis californicus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 90_000.0,
    description: "Small western Myotis, highly adaptable. FM sweep with characteristic frequency ~50 kHz. Difficult to distinguish acoustically from Western Small-footed Myotis. Often the most common bat at western sites.",
    echolocates: true,
};

pub const MYOTIS_CILIOLABRUM: BatSpecies = BatSpecies {
    id: "myotis_ciliolabrum",
    name: "Western Small-footed Myotis",
    scientific_name: "Myotis ciliolabrum",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 90_000.0,
    description: "Tiny bat (~5 g) of western arid lands. FM sweep with characteristic frequency ~50 kHz. Calls nearly identical to California Myotis. Roosts in rock crevices, cliff faces, and badlands.",
    echolocates: true,
};

pub const MYOTIS_AUSTRORIPARIUS: BatSpecies = BatSpecies {
    id: "myotis_austroriparius",
    name: "Southeastern Myotis",
    scientific_name: "Myotis austroriparius",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 80_000.0,
    description: "Southeastern US cave bat, often found near water. FM sweep with characteristic frequency ~50 kHz. Forms large maternity colonies in caves, sometimes with thousands of individuals.",
    echolocates: true,
};

pub const MYOTIS_YUMANENSIS: BatSpecies = BatSpecies {
    id: "myotis_yumanensis",
    name: "Yuma Myotis",
    scientific_name: "Myotis yumanensis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 80_000.0,
    description: "Western water-foraging specialist. FM sweep with characteristic frequency ~50 kHz. Trawls insects from water surfaces with large feet. Common near rivers and lakes.",
    echolocates: true,
};

pub const MYOTIS_VELIFER: BatSpecies = BatSpecies {
    id: "myotis_velifer",
    name: "Cave Myotis",
    scientific_name: "Myotis velifer",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 70_000.0,
    description: "Large Myotis of the south-central US and Mexico. FM sweep with characteristic frequency ~45 kHz. Forms large cave colonies. Bare patches on back between shoulder blades diagnostic.",
    echolocates: true,
};

pub const LASIURUS_BOREALIS: BatSpecies = BatSpecies {
    id: "lasiurus_borealis",
    name: "Eastern Red Bat",
    scientific_name: "Lasiurus borealis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 55_000.0,
    description: "Distinctive reddish fur. Solitary foliage-roosting tree bat. FM sweep with characteristic frequency ~40 kHz. Long-distance migrant. One of the most common bats in eastern North America.",
    echolocates: true,
};

pub const LASIURUS_CINEREUS: BatSpecies = BatSpecies {
    id: "lasiurus_cinereus",
    name: "Hoary Bat",
    scientific_name: "Lasiurus cinereus",
    family: "Vespertilionidae",
    call_type: "QCF",
    freq_lo_hz: 18_000.0,
    freq_hi_hz: 28_000.0,
    description: "North America's largest bat (~30 g). Distinctive low-frequency QCF calls ~20\u{2013}25 kHz, easily identified on spectrograms. Long-distance migrant, solitary foliage rooster. Frosted brown fur with white-tipped hairs.",
    echolocates: true,
};

pub const LASIURUS_SEMINOLUS: BatSpecies = BatSpecies {
    id: "lasiurus_seminolus",
    name: "Seminole Bat",
    scientific_name: "Lasiurus seminolus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 50_000.0,
    description: "Southeastern counterpart of the Eastern Red Bat with deep mahogany fur. FM sweep with characteristic frequency ~40 kHz. Roosts in Spanish moss and pine foliage.",
    echolocates: true,
};

pub const LASIURUS_INTERMEDIUS: BatSpecies = BatSpecies {
    id: "lasiurus_intermedius",
    name: "Northern Yellow Bat",
    scientific_name: "Lasiurus intermedius",
    family: "Vespertilionidae",
    call_type: "QCF",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 40_000.0,
    description: "Large yellowish tree bat of the southeastern coastal plain. FM-QCF calls with characteristic frequency ~30 kHz. Roosts in dead palm fronds and Spanish moss.",
    echolocates: true,
};

pub const LASIURUS_XANTHINUS: BatSpecies = BatSpecies {
    id: "lasiurus_xanthinus",
    name: "Western Yellow Bat",
    scientific_name: "Lasiurus xanthinus",
    family: "Vespertilionidae",
    call_type: "QCF",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 40_000.0,
    description: "Southwestern desert species. FM-QCF calls with characteristic frequency ~30 kHz. Roosts in palm fronds. Range expanding northward with ornamental palm plantings.",
    echolocates: true,
};

pub const LASIONYCTERIS_NOCTIVAGANS: BatSpecies = BatSpecies {
    id: "lasionycteris_noctivagans",
    name: "Silver-haired Bat",
    scientific_name: "Lasionycteris noctivagans",
    family: "Vespertilionidae",
    call_type: "QCF",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 35_000.0,
    description: "Medium-sized bat with distinctive silver-tipped dark fur. Low-frequency QCF calls ~25\u{2013}27 kHz characteristic frequency. Slow, maneuverable flight. Solitary tree-roosting migrant. One of the most frequently killed bats at wind turbines.",
    echolocates: true,
};

pub const PERIMYOTIS_SUBFLAVUS: BatSpecies = BatSpecies {
    id: "perimyotis_subflavus",
    name: "Tricolored Bat",
    scientific_name: "Perimyotis subflavus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 55_000.0,
    description: "Tiny bat (~6 g) formerly called Eastern Pipistrelle. FM sweep with characteristic frequency ~45 kHz. Distinctive tricolored fur bands. Severely affected by White-nose Syndrome; proposed for ESA listing.",
    echolocates: true,
};

pub const PARASTRELLUS_HESPERUS: BatSpecies = BatSpecies {
    id: "parastrellus_hesperus",
    name: "Canyon Bat",
    scientific_name: "Parastrellus hesperus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 43_000.0,
    freq_hi_hz: 70_000.0,
    description: "Smallest bat in North America (~3.5 g), formerly called Western Pipistrelle. FM sweep with characteristic frequency ~50 kHz. Common in desert canyons and rock outcrops. Often the first bat seen at dusk.",
    echolocates: true,
};

pub const NYCTICEIUS_HUMERALIS: BatSpecies = BatSpecies {
    id: "nycticeius_humeralis",
    name: "Evening Bat",
    scientific_name: "Nycticeius humeralis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 45_000.0,
    description: "Medium-sized bat of the eastern US. FM-QCF calls with characteristic frequency ~35 kHz. Resembles Big Brown Bat but smaller. Roosts in tree cavities and buildings. Does not use caves.",
    echolocates: true,
};

pub const CORYNORHINUS_TOWNSENDII: BatSpecies = BatSpecies {
    id: "corynorhinus_townsendii",
    name: "Townsend's Big-eared Bat",
    scientific_name: "Corynorhinus townsendii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 50_000.0,
    description: "Enormous ears (~35 mm). Very quiet, short FM calls for gleaning moths from foliage. Characteristic frequency ~25\u{2013}40 kHz. Highly sensitive to roost disturbance. Several subspecies are endangered.",
    echolocates: true,
};

pub const CORYNORHINUS_RAFINESQUII: BatSpecies = BatSpecies {
    id: "corynorhinus_rafinesquii",
    name: "Rafinesque's Big-eared Bat",
    scientific_name: "Corynorhinus rafinesquii",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 50_000.0,
    description: "Southeastern counterpart of Townsend's Big-eared Bat. Very quiet gleaning calls, characteristic frequency ~25\u{2013}40 kHz. Roosts in abandoned buildings, hollow trees, and under bridges. White belly fur distinctive.",
    echolocates: true,
};

pub const IDIONYCTERIS_PHYLLOTIS: BatSpecies = BatSpecies {
    id: "idionycteris_phyllotis",
    name: "Allen's Big-eared Bat",
    scientific_name: "Idionycteris phyllotis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 9_000.0,
    freq_hi_hz: 25_000.0,
    description: "Distinctive low-frequency calls ~12\u{2013}15 kHz, often audible to humans. Large lappets projecting from base of ears. Southwestern pine and oak forests. Calls unlike any other NA Myotis-sized bat.",
    echolocates: true,
};

pub const EUDERMA_MACULATUM: BatSpecies = BatSpecies {
    id: "euderma_maculatum",
    name: "Spotted Bat",
    scientific_name: "Euderma maculatum",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 8_000.0,
    freq_hi_hz: 15_000.0,
    description: "Lowest echolocation frequency of any North American bat (~10\u{2013}12 kHz), audible to humans. Unmistakable: three large white spots on black dorsal fur, enormous pink ears. Rare but widespread in western arid lands.",
    echolocates: true,
};

pub const ANTROZOUS_PALLIDUS: BatSpecies = BatSpecies {
    id: "antrozous_pallidus",
    name: "Pallid Bat",
    scientific_name: "Antrozous pallidus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 28_000.0,
    freq_hi_hz: 55_000.0,
    description: "Unique dual foraging strategy: echolocates for aerial prey and uses passive listening to glean scorpions and large insects from the ground. FM calls ~30 kHz characteristic. Large ears, pale fur. Immune to scorpion venom.",
    echolocates: true,
};

// ── Molossidae ───────────────────────────────────────────────────────────────

pub const NYCTINOMOPS_MACROTIS: BatSpecies = BatSpecies {
    id: "nyctinomops_macrotis",
    name: "Big Free-tailed Bat",
    scientific_name: "Nyctinomops macrotis",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 10_000.0,
    freq_hi_hz: 20_000.0,
    description: "Large molossid with low-frequency QCF calls ~14\u{2013}17 kHz, often audible. Roosts in high cliff crevices. Southwestern US. Loud, piercing social calls can be heard from considerable distance.",
    echolocates: true,
};

pub const NYCTINOMOPS_FEMOROSACCUS: BatSpecies = BatSpecies {
    id: "nyctinomops_femorosaccus",
    name: "Pocketed Free-tailed Bat",
    scientific_name: "Nyctinomops femorosaccus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 16_000.0,
    freq_hi_hz: 25_000.0,
    description: "Medium-sized free-tailed bat of the southwestern deserts. QCF calls ~22 kHz characteristic frequency. Named for the pocket-like fold on the tail membrane. Roosts in rock crevices.",
    echolocates: true,
};

pub const EUMOPS_PEROTIS: BatSpecies = BatSpecies {
    id: "eumops_perotis",
    name: "Western Mastiff Bat",
    scientific_name: "Eumops perotis",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 7_000.0,
    freq_hi_hz: 18_000.0,
    description: "Largest bat in North America (wingspan ~56 cm). Very low-frequency QCF calls ~10\u{2013}12 kHz, easily audible to humans. Needs vertical drop to launch into flight. Roosts in tall cliff faces and buildings.",
    echolocates: true,
};

pub const EUMOPS_FLORIDANUS: BatSpecies = BatSpecies {
    id: "eumops_floridanus",
    name: "Florida Bonneted Bat",
    scientific_name: "Eumops floridanus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 8_000.0,
    freq_hi_hz: 20_000.0,
    description: "Federally endangered, restricted to southern Florida. Low-frequency QCF calls ~14\u{2013}16 kHz. One of the rarest bats in North America. Roosts in tree cavities and bat houses. Occasionally audible to humans.",
    echolocates: true,
};

// ── Phyllostomidae ───────────────────────────────────────────────────────────

pub const MACROTUS_CALIFORNICUS: BatSpecies = BatSpecies {
    id: "macrotus_californicus",
    name: "California Leaf-nosed Bat",
    scientific_name: "Macrotus californicus",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 90_000.0,
    description: "Low-intensity gleaning calls ~50 kHz, difficult to detect with bat detectors. Large ears and prominent nose leaf. Non-migratory desert species of AZ and southern CA. Cannot hibernate; relies on warm roost sites year-round.",
    echolocates: true,
};

pub const CHOERONYCTERIS_MEXICANA: BatSpecies = BatSpecies {
    id: "choeronycteris_mexicana",
    name: "Mexican Long-tongued Bat",
    scientific_name: "Choeronycteris mexicana",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 90_000.0,
    description: "Nectar-feeding bat with elongated snout and long tongue. Very quiet FM calls ~75 kHz. Seasonal migrant to southern AZ. Pollinates agave and columnar cacti. Low-intensity echolocation typical of phyllostomids.",
    echolocates: true,
};

pub const LEPTONYCTERIS_YERBABUENAE: BatSpecies = BatSpecies {
    id: "leptonycteris_yerbabuenae",
    name: "Lesser Long-nosed Bat",
    scientific_name: "Leptonycteris yerbabuenae",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 90_000.0,
    description: "Nectar bat, major pollinator of saguaro and organ pipe cacti. Quiet FM calls ~75 kHz. Migrates to southern AZ in summer. Formerly endangered, delisted in 2018 \u{2014} a conservation success story.",
    echolocates: true,
};

pub const LEPTONYCTERIS_NIVALIS: BatSpecies = BatSpecies {
    id: "leptonycteris_nivalis",
    name: "Mexican Long-nosed Bat",
    scientific_name: "Leptonycteris nivalis",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 65_000.0,
    freq_hi_hz: 85_000.0,
    description: "Federally endangered nectar bat. Migrates to Big Bend region of Texas in summer. Pollinates agave plants. Quiet FM calls ~75 kHz. Only known US roost in a single cave in the Chisos Mountains.",
    echolocates: true,
};

// ══════════════════════════════════════════════════════════════════════════════
// South America additional species
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - López-Baucells et al. (2016) Pelagic: Guía de los murciélagos de la Amazonia
// - Barataud (2020): Ecologie acoustique des chiroptères d'Europe (neotropical appendix)
// - Jung et al. (2014) PMC: Neotropical molossid call design
// - Arias-Aguilar et al. (2018): Echolocation of Amazonian bats
// - Schnitzler & Kalko (2001): Echolocation by insect-eating bats
// - Nogueira et al. (2014): Echolocation of Brazilian bats
// - Falcão et al. (2015): Bat assemblages in Brazilian Atlantic Forest
// - Rodríguez-San Pedro & Simonetti (2015): Chilean bats

// ── Mormoopidae ────────────────────────────────────────────────────────────

pub const PTERONOTUS_PARNELLII: BatSpecies = BatSpecies {
    id: "pteronotus_parnellii",
    name: "Parnell's Mustached Bat",
    scientific_name: "Pteronotus parnellii",
    family: "Mormoopidae",
    call_type: "CF-FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 120_000.0,
    description: "The South American high-duty-cycle echolocator. Long CF at ~60 kHz (2nd harmonic) with Doppler compensation. Split from P. mesoamericanus. Huge cave colonies.",
    echolocates: true,
};

// ── Furipteridae (endemic Neotropical family) ──────────────────────────────

pub const FURIPTERUS_HORRENS: BatSpecies = BatSpecies {
    id: "furipterus_horrens",
    name: "Thumbless Bat",
    scientific_name: "Furipterus horrens",
    family: "Furipteridae",
    call_type: "FM",
    freq_lo_hz: 90_000.0,
    freq_hi_hz: 160_000.0,
    description: "Tiny (3\u{2013}5 g) insectivore with vestigial thumb. Very high frequency FM sweeps peaking ~130\u{2013}150 kHz. One of the highest-frequency New World bats. Caves and mines.",
    echolocates: true,
};

pub const AMORPHOCHILUS_SCHNABLII: BatSpecies = BatSpecies {
    id: "amorphochilus_schnablii",
    name: "Smoky Bat",
    scientific_name: "Amorphochilus schnablii",
    family: "Furipteridae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 120_000.0,
    description: "Rare, endemic to western South America (Ecuador, Peru, Chile). High-frequency FM sweeps ~80\u{2013}100 kHz. Cave-dwelling. IUCN Vulnerable.",
    echolocates: true,
};

// ── Natalidae ──────────────────────────────────────────────────────────────

pub const NATALUS_MACROURUS: BatSpecies = BatSpecies {
    id: "natalus_macrourus",
    name: "Brazilian Funnel-eared Bat",
    scientific_name: "Natalus macrourus",
    family: "Natalidae",
    call_type: "FM",
    freq_lo_hz: 80_000.0,
    freq_hi_hz: 150_000.0,
    description: "Very high frequency echolocator, peak ~100\u{2013}120 kHz. Extremely low intensity. Cave-roosting. Endemic to eastern Brazil (Cerrado/Caatinga). IUCN Vulnerable.",
    echolocates: true,
};

pub const NATALUS_TUMIDIROSTRIS: BatSpecies = BatSpecies {
    id: "natalus_tumidirostris",
    name: "Trinidadian Funnel-eared Bat",
    scientific_name: "Natalus tumidirostris",
    family: "Natalidae",
    call_type: "FM",
    freq_lo_hz: 80_000.0,
    freq_hi_hz: 140_000.0,
    description: "Very high frequency FM, peak ~100\u{2013}130 kHz. Cave-roosting. Northern South America (Venezuela, Colombia, Trinidad). Low-intensity calls barely detectable beyond 1 m.",
    echolocates: true,
};

// ── Thyropteridae ──────────────────────────────────────────────────────────

pub const THYROPTERA_DISCIFERA: BatSpecies = BatSpecies {
    id: "thyroptera_discifera",
    name: "Peters' Disk-winged Bat",
    scientific_name: "Thyroptera discifera",
    family: "Thyropteridae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 80_000.0,
    description: "Similar to T. tricolor but slightly larger suction disks. Peak ~50 kHz. Roosts in furled Heliconia leaves. Amazonian lowland forests. Extremely low-intensity echolocation.",
    echolocates: true,
};

pub const THYROPTERA_LAVALI: BatSpecies = BatSpecies {
    id: "thyroptera_lavali",
    name: "LaVal's Disk-winged Bat",
    scientific_name: "Thyroptera lavali",
    family: "Thyropteridae",
    call_type: "FM",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 75_000.0,
    description: "Rare Amazonian disk-winged bat. Roosts in curled leaves. Poorly known acoustically. Low-intensity FM calls.",
    echolocates: true,
};

// ── Emballonuridae ─────────────────────────────────────────────────────────

pub const SACCOPTERYX_CANESCENS: BatSpecies = BatSpecies {
    id: "saccopteryx_canescens",
    name: "Frosted Sac-winged Bat",
    scientific_name: "Saccopteryx canescens",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 48_000.0,
    freq_hi_hz: 55_000.0,
    description: "Amazonian endemic. Highest frequency Saccopteryx (~52 kHz). Similar QCF structure to congeners. Smaller than S. bilineata. Roosts on tree trunks in terra firme forest.",
    echolocates: true,
};

pub const CENTRONYCTERIS_MAXIMILIANI: BatSpecies = BatSpecies {
    id: "centronycteris_maximiliani",
    name: "Shaggy Bat",
    scientific_name: "Centronycteris maximiliani",
    family: "Emballonuridae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 65_000.0,
    description: "Rare canopy-level forager. Steep FM sweeps ~45\u{2013}55 kHz. Long shaggy fur. One of the least-known Neotropical emballonurids. Amazonian forests.",
    echolocates: true,
};

pub const PEROPTERYX_LEUCOPTERA: BatSpecies = BatSpecies {
    id: "peropteryx_leucoptera",
    name: "White-winged Dog-like Bat",
    scientific_name: "Peropteryx leucoptera",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 38_000.0,
    freq_hi_hz: 50_000.0,
    description: "Distinctive white wing membrane patches. QCF at ~42 kHz. Amazonian lowland forests.",
    echolocates: true,
};

pub const PEROPTERYX_TRINITATIS: BatSpecies = BatSpecies {
    id: "peropteryx_trinitatis",
    name: "Trinidad Dog-like Bat",
    scientific_name: "Peropteryx trinitatis",
    family: "Emballonuridae",
    call_type: "QCF",
    freq_lo_hz: 38_000.0,
    freq_hi_hz: 52_000.0,
    description: "Northern South America. QCF at ~43 kHz. Open-area forager near rock shelters and caves.",
    echolocates: true,
};

// ── Phyllostomidae (additional South American species) ─────────────────────

pub const DIPHYLLA_ECAUDATA: BatSpecies = BatSpecies {
    id: "diphylla_ecaudata",
    name: "Hairy-legged Vampire Bat",
    scientific_name: "Diphylla ecaudata",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 65_000.0,
    freq_hi_hz: 100_000.0,
    description: "Bird blood specialist (unlike the mammal-feeding D. rotundus). Peak ~80 kHz. Higher frequency than common vampire. Short multiharmonic FM. Low-intensity.",
    echolocates: true,
};

pub const DIAEMUS_YOUNGI: BatSpecies = BatSpecies {
    id: "diaemus_youngi",
    name: "White-winged Vampire Bat",
    scientific_name: "Diaemus youngi",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 95_000.0,
    description: "Feeds on bird blood. Peak ~70 kHz. White wing tips distinctive in flight. Low-intensity FM. Rarer than D. rotundus. Lowland forests.",
    echolocates: true,
};

pub const ARTIBEUS_OBSCURUS: BatSpecies = BatSpecies {
    id: "artibeus_obscurus",
    name: "Dark Fruit-eating Bat",
    scientific_name: "Artibeus obscurus",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 85_000.0,
    description: "Common Amazonian frugivore. Peak ~55 kHz. Intermediate size between A. jamaicensis and A. lituratus. Low-intensity FM. Indistinct facial stripes.",
    echolocates: true,
};

pub const ARTIBEUS_PLANIROSTRIS: BatSpecies = BatSpecies {
    id: "artibeus_planirostris",
    name: "Flat-faced Fruit Bat",
    scientific_name: "Artibeus planirostris",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 48_000.0,
    freq_hi_hz: 82_000.0,
    description: "Very common South American frugivore, replaces A. jamaicensis in many mainland habitats. Peak ~54 kHz. Low-intensity multiharmonic FM. Important seed disperser.",
    echolocates: true,
};

pub const PLATYRRHINUS_LINEATUS: BatSpecies = BatSpecies {
    id: "platyrrhinus_lineatus",
    name: "White-lined Broad-nosed Bat",
    scientific_name: "Platyrrhinus lineatus",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 90_000.0,
    description: "Common frugivore of open habitats and forest edges. Peak ~62 kHz. Prominent white facial and dorsal stripes. Low-intensity FM. Cerrado and Atlantic Forest.",
    echolocates: true,
};

pub const PLATYRRHINUS_HELLERI: BatSpecies = BatSpecies {
    id: "platyrrhinus_helleri",
    name: "Heller's Broad-nosed Bat",
    scientific_name: "Platyrrhinus helleri",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 95_000.0,
    description: "Small frugivore. Peak ~68 kHz. Low-intensity FM. Widespread in Neotropical lowland forests. Important seed disperser.",
    echolocates: true,
};

pub const VAMPYRESSA_THYONE: BatSpecies = BatSpecies {
    id: "vampyressa_thyone",
    name: "Northern Little Yellow-eared Bat",
    scientific_name: "Vampyressa thyone",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 105_000.0,
    description: "Small frugivore. Peak ~78 kHz. Low-intensity FM. Tent-roosting. Yellow ear margins. Northern South American lowlands.",
    echolocates: true,
};

pub const VAMPYRODES_CARACCIOLI: BatSpecies = BatSpecies {
    id: "vampyrodes_caraccioli",
    name: "Great Stripe-faced Bat",
    scientific_name: "Vampyrodes caraccioli",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 48_000.0,
    freq_hi_hz: 82_000.0,
    description: "Large stenodermatine frugivore. Peak ~58 kHz. Prominent white facial stripes. Low-intensity FM. Forages in canopy.",
    echolocates: true,
};

pub const LONCHOPHYLLA_THOMASI: BatSpecies = BatSpecies {
    id: "lonchophylla_thomasi",
    name: "Thomas's Nectar Bat",
    scientific_name: "Lonchophylla thomasi",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 110_000.0,
    description: "Small nectarivore. Peak ~80 kHz. Low-intensity multiharmonic FM. Important pollinator of understory plants. Amazonian forests.",
    echolocates: true,
};

pub const ANOURA_CAUDIFER: BatSpecies = BatSpecies {
    id: "anoura_caudifer",
    name: "Tailed Tailless Bat",
    scientific_name: "Anoura caudifer",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 100_000.0,
    description: "Nectarivore with short tail (unlike tailless A. geoffroyi). Peak ~72 kHz. Low-intensity FM. Atlantic Forest and lower Andean slopes. Important pollinator.",
    echolocates: true,
};

pub const ANOURA_CULTRATA: BatSpecies = BatSpecies {
    id: "anoura_cultrata",
    name: "Handley's Tailless Bat",
    scientific_name: "Anoura cultrata",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 55_000.0,
    freq_hi_hz: 95_000.0,
    description: "Highland nectarivore. Peak ~68 kHz. Low-intensity FM. Andean cloud forests 1000\u{2013}2600 m. Uniquely keeled lower incisors.",
    echolocates: true,
};

pub const STURNIRA_ERYTHROMOS: BatSpecies = BatSpecies {
    id: "sturnira_erythromos",
    name: "Hairy Yellow-shouldered Bat",
    scientific_name: "Sturnira erythromos",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 62_000.0,
    freq_hi_hz: 80_000.0,
    description: "Andean frugivore of montane forests (1000\u{2013}3200 m). Peak ~70 kHz. Low-intensity FM. Key seed disperser in cloud forest ecosystems.",
    echolocates: true,
};

pub const STURNIRA_BOGOTENSIS: BatSpecies = BatSpecies {
    id: "sturnira_bogotensis",
    name: "Bogota Yellow-shouldered Bat",
    scientific_name: "Sturnira bogotensis",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 60_000.0,
    freq_hi_hz: 78_000.0,
    description: "High-altitude Andean frugivore (2000\u{2013}3400 m). Peak ~68 kHz. Low-intensity FM. One of the few bat species found above 3000 m.",
    echolocates: true,
};

pub const CHIRODERMA_VILLOSUM: BatSpecies = BatSpecies {
    id: "chiroderma_villosum",
    name: "Hairy Big-eyed Bat",
    scientific_name: "Chiroderma villosum",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 50_000.0,
    freq_hi_hz: 90_000.0,
    description: "Canopy frugivore with large eyes. Peak ~62 kHz. Low-intensity FM. Widespread in Neotropical lowlands. White dorsal stripe.",
    echolocates: true,
};

pub const PHYLLODERMA_STENOPS: BatSpecies = BatSpecies {
    id: "phylloderma_stenops",
    name: "Pale-faced Bat",
    scientific_name: "Phylloderma stenops",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 45_000.0,
    freq_hi_hz: 80_000.0,
    description: "Large omnivorous phyllostomid. Peak ~55 kHz. Low-intensity FM. Pale face distinctive. Roosts in hollow trees. Amazonian forests.",
    echolocates: true,
};

pub const RHINOPHYLLA_PUMILIO: BatSpecies = BatSpecies {
    id: "rhinophylla_pumilio",
    name: "Dwarf Little Fruit Bat",
    scientific_name: "Rhinophylla pumilio",
    family: "Phyllostomidae",
    call_type: "FM",
    freq_lo_hz: 65_000.0,
    freq_hi_hz: 110_000.0,
    description: "Small Amazonian frugivore. Peak ~80 kHz. Low-intensity FM. Related to Carollia. Important disperser of understory fruits. Common in terra firme forest.",
    echolocates: true,
};

// ── Molossidae (additional South American species) ─────────────────────────

pub const MOLOSSOPS_TEMMINCKII: BatSpecies = BatSpecies {
    id: "molossops_temminckii",
    name: "Dwarf Dog-faced Bat",
    scientific_name: "Molossops temminckii",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 30_000.0,
    freq_hi_hz: 50_000.0,
    description: "Small molossid. QCF at ~38\u{2013}42 kHz. One of the highest frequency molossids, consistent with small body size. Open cerrado and forest edge forager.",
    echolocates: true,
};

pub const MOLOSSOPS_NEGLECTUS: BatSpecies = BatSpecies {
    id: "molossops_neglectus",
    name: "Rufous Dog-faced Bat",
    scientific_name: "Molossops neglectus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 32_000.0,
    freq_hi_hz: 48_000.0,
    description: "Small Amazonian molossid. QCF at ~40 kHz. Poorly known. Forest edge and gap forager.",
    echolocates: true,
};

pub const EUMOPS_BONARIENSIS: BatSpecies = BatSpecies {
    id: "eumops_bonariensis",
    name: "Peters' Mastiff Bat",
    scientific_name: "Eumops bonariensis",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 34_000.0,
    description: "Medium-sized bonneted bat. QCF at ~26\u{2013}30 kHz. Open-area forager. Southern South America (Argentina, Uruguay, Brazil). Roosts in buildings.",
    echolocates: true,
};

pub const EUMOPS_HANSAE: BatSpecies = BatSpecies {
    id: "eumops_hansae",
    name: "Sanborn's Bonneted Bat",
    scientific_name: "Eumops hansae",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 24_000.0,
    freq_hi_hz: 38_000.0,
    description: "Medium molossid. QCF at ~28\u{2013}32 kHz. Amazonian and Atlantic Forest lowlands. Roosts in tree hollows. Uncommonly recorded.",
    echolocates: true,
};

pub const EUMOPS_DABBENEI: BatSpecies = BatSpecies {
    id: "eumops_dabbenei",
    name: "Big Bonneted Bat",
    scientific_name: "Eumops dabbenei",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 14_000.0,
    freq_hi_hz: 24_000.0,
    description: "Very large molossid. Low-frequency QCF at ~18\u{2013}20 kHz, sometimes audible. Open-space forager over savanna and cerrado. Northern Argentina to Colombia.",
    echolocates: true,
};

pub const EUMOPS_PATAGONICUS: BatSpecies = BatSpecies {
    id: "eumops_patagonicus",
    name: "Patagonian Bonneted Bat",
    scientific_name: "Eumops patagonicus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 22_000.0,
    freq_hi_hz: 36_000.0,
    description: "Medium bonneted bat of the southern cone (Argentina, Paraguay, southern Brazil). QCF at ~28 kHz. Open and semi-open habitat forager. Roosts in buildings and tree hollows.",
    echolocates: true,
};

pub const PROMOPS_NASUTUS: BatSpecies = BatSpecies {
    id: "promops_nasutus",
    name: "Brown Mastiff Bat",
    scientific_name: "Promops nasutus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 25_000.0,
    freq_hi_hz: 38_000.0,
    description: "QCF at ~30 kHz. Similar to P. centralis but lacks the distinctive upward frequency modulation. Open-space forager. South American drylands and forest edges.",
    echolocates: true,
};

pub const CYNOMOPS_ABRASUS: BatSpecies = BatSpecies {
    id: "cynomops_abrasus",
    name: "Cinnamon Dog-faced Bat",
    scientific_name: "Cynomops abrasus",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 18_000.0,
    freq_hi_hz: 28_000.0,
    description: "Medium molossid. Low-frequency QCF at ~22\u{2013}24 kHz. Open-space forager. Brazilian cerrado and Atlantic Forest edge.",
    echolocates: true,
};

pub const CYNOMOPS_PLANIROSTRIS: BatSpecies = BatSpecies {
    id: "cynomops_planirostris",
    name: "Southern Dog-faced Bat",
    scientific_name: "Cynomops planirostris",
    family: "Molossidae",
    call_type: "QCF",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 32_000.0,
    description: "Small to medium molossid. QCF at ~25 kHz. Flat face with forward-pointing nostrils. Open habitats across South American lowlands.",
    echolocates: true,
};

// ── Vespertilionidae (additional South American species) ───────────────────

pub const MYOTIS_ALBESCENS: BatSpecies = BatSpecies {
    id: "myotis_albescens",
    name: "Silver-tipped Myotis",
    scientific_name: "Myotis albescens",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 38_000.0,
    freq_hi_hz: 80_000.0,
    description: "Common South American Myotis. Peak ~48 kHz. FM sweeps. Silver-tipped dorsal fur distinctive. Forages over water and in forest clearings.",
    echolocates: true,
};

pub const MYOTIS_CHILOENSIS: BatSpecies = BatSpecies {
    id: "myotis_chiloensis",
    name: "Chilean Myotis",
    scientific_name: "Myotis chiloensis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 35_000.0,
    freq_hi_hz: 75_000.0,
    description: "Southern South American Myotis (Chile, Argentina, Patagonia). Peak ~45 kHz. FM sweeps. One of the southernmost-ranging bats in the Americas.",
    echolocates: true,
};

pub const MYOTIS_LEVIS: BatSpecies = BatSpecies {
    id: "myotis_levis",
    name: "Yellowish Myotis",
    scientific_name: "Myotis levis",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 38_000.0,
    freq_hi_hz: 75_000.0,
    description: "South American Myotis. Peak ~46 kHz. FM sweeps. Southern Brazil, Uruguay, Argentina. Open areas and forest edges.",
    echolocates: true,
};

pub const MYOTIS_OXYOTUS: BatSpecies = BatSpecies {
    id: "myotis_oxyotus",
    name: "Montane Myotis",
    scientific_name: "Myotis oxyotus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 80_000.0,
    description: "High-altitude Andean Myotis (1500\u{2013}3800 m). Peak ~50 kHz. FM sweeps. Cloud forests and paramo edges. Colombia to Bolivia.",
    echolocates: true,
};

pub const MYOTIS_LAVALI: BatSpecies = BatSpecies {
    id: "myotis_lavali",
    name: "LaVal's Myotis",
    scientific_name: "Myotis lavali",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 42_000.0,
    freq_hi_hz: 85_000.0,
    description: "Small Myotis endemic to eastern Brazil (Cerrado, Caatinga). Peak ~52 kHz. Steep FM sweeps. Recently described. Associated with rock outcrops.",
    echolocates: true,
};

pub const LASIURUS_VILLOSISSIMUS: BatSpecies = BatSpecies {
    id: "lasiurus_villosissimus",
    name: "South American Hoary Bat",
    scientific_name: "Lasiurus villosissimus",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 18_000.0,
    freq_hi_hz: 35_000.0,
    description: "Large vespertilionid, recently split from L. cinereus. Low-frequency FM-QCF at ~22\u{2013}25 kHz. Long narrow wings for fast open-air flight. Migratory. Frosted fur.",
    echolocates: true,
};

pub const HISTIOTUS_MONTANUS: BatSpecies = BatSpecies {
    id: "histiotus_montanus",
    name: "Small Big-eared Brown Bat",
    scientific_name: "Histiotus montanus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 18_000.0,
    freq_hi_hz: 45_000.0,
    description: "Distinctive very large ears (Plecotus-like). Peak ~28 kHz. FM sweeps. Open habitats in southern South America (Patagonia to southern Brazil). Slow, maneuverable flight.",
    echolocates: true,
};

pub const HISTIOTUS_VELATUS: BatSpecies = BatSpecies {
    id: "histiotus_velatus",
    name: "Tropical Big-eared Brown Bat",
    scientific_name: "Histiotus velatus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 20_000.0,
    freq_hi_hz: 50_000.0,
    description: "Large-eared vespertilionid. Peak ~32 kHz. FM sweeps. Brazilian cerrado and Atlantic Forest. Slightly higher frequency than H. montanus, consistent with smaller ears.",
    echolocates: true,
};

pub const HISTIOTUS_MACROTUS: BatSpecies = BatSpecies {
    id: "histiotus_macrotus",
    name: "Big-eared Brown Bat",
    scientific_name: "Histiotus macrotus",
    family: "Vespertilionidae",
    call_type: "FM",
    freq_lo_hz: 16_000.0,
    freq_hi_hz: 40_000.0,
    description: "Largest-eared Histiotus. Very low-frequency peak ~25 kHz. FM sweeps. Chile and western Argentina. Arid and semi-arid open habitats. Gleans insects from foliage.",
    echolocates: true,
};

pub const EPTESICUS_DIMINUTUS: BatSpecies = BatSpecies {
    id: "eptesicus_diminutus",
    name: "Diminutive Serotine",
    scientific_name: "Eptesicus diminutus",
    family: "Vespertilionidae",
    call_type: "FM-QCF",
    freq_lo_hz: 40_000.0,
    freq_hi_hz: 70_000.0,
    description: "Small Eptesicus of southern Brazil, Paraguay, Argentina. Peak ~50 kHz. FM-QCF. Forest edges and open areas.",
    echolocates: true,
};
