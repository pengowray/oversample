use super::types::{BookEntryDef, BatBookManifest, BatBookRegion, Commonness};
use super::species;

// ══════════════════════════════════════════════════════════════════════════════
// Global book — family-level entries
// ══════════════════════════════════════════════════════════════════════════════

const GLOBAL_BOOK: &[BookEntryDef] = &[
    BookEntryDef { species: &species::RHINOLOPHIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::HIPPOSIDERIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::VESPERTILIONIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::MOLOSSIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::EMBALLONURIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::PHYLLOSTOMIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::MORMOOPIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::MINIOPTERIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::NYCTERIDAE, commonness: None, description: None, name: None },
    BookEntryDef { species: &species::MEGADERMATIDAE, commonness: None, description: None, name: None },
    // Non-echolocating (will be sorted to end by get_manifest)
    BookEntryDef { species: &species::PTEROPODIDAE, commonness: None, description: None, name: None },
];

// ══════════════════════════════════════════════════════════════════════════════
// VIC, Australia — species-level entries sorted by commonness
// ══════════════════════════════════════════════════════════════════════════════

const VIC_AUSTRALIA_BOOK: &[BookEntryDef] = &[
    // ── Very Common ──────────────────────────────────────────────
    BookEntryDef {
        species: &species::CHALINOLOBUS_GOULDII,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Widespread and abundant across Victoria. Roosts in tree hollows, buildings, and bat boxes. Alternating call frequencies are distinctive."),
        name: None,
    },
    BookEntryDef {
        species: &species::CHALINOLOBUS_MORIO,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Common across southern Australia. Small, dark bat roosting in tree hollows and buildings. Higher frequency calls than Gould's Wattled Bat."),
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTOPHILUS_GEOFFROYI,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Australia's most widespread bat. Very quiet, broadband FM calls; often difficult to detect acoustically. Gleaning insectivore with large ears."),
        name: None,
    },
    BookEntryDef {
        species: &species::AUSTRONOMUS_AUSTRALIS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Australia's largest insectivorous bat. Loud, low-frequency calls audible to some humans. Fast, high-flying open-air forager."),
        name: None,
    },
    BookEntryDef {
        species: &species::VESPADELUS_VULTURNUS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("One of Australia's smallest bats (~4 g). Common in forests and urban areas throughout Victoria. High-frequency calls."),
        name: None,
    },
    // ── Common ───────────────────────────────────────────────────
    BookEntryDef {
        species: &species::VESPADELUS_REGULUS,
        commonness: Some(Commonness::Common),
        description: Some("Small forest bat found across southern Australia. Roosts in tree hollows. Call frequency overlaps with Little Forest Bat."),
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTOPHILUS_GOULDI,
        commonness: Some(Commonness::Common),
        description: Some("Common in forests of eastern Australia. Very quiet calls, similar to Lesser Long-eared Bat. Distinguished by larger size and habitat preference."),
        name: None,
    },
    BookEntryDef {
        species: &species::VESPADELUS_DARLINGTONI,
        commonness: Some(Commonness::Common),
        description: Some("Largest Vespadelus species. Found in wet and dry forests of south-eastern Australia including Tasmania."),
        name: None,
    },
    BookEntryDef {
        species: &species::MINIOPTERUS_ORIANAE_OCEANENSIS,
        commonness: Some(Commonness::Common),
        description: Some("Cave-roosting bat found along eastern Australia. Fast, agile flier. Maternity cave near Bairnsdale. Vulnerable in Victoria."),
        name: None,
    },
    BookEntryDef {
        species: &species::OZIMOPS_PLANICEPS,
        commonness: Some(Commonness::Common),
        description: Some("Small free-tailed bat of south-eastern Australia. Roosts in tree hollows and buildings. Rapid, direct flight."),
        name: None,
    },
    BookEntryDef {
        species: &species::OZIMOPS_RIDEI,
        commonness: Some(Commonness::Common),
        description: Some("Widespread across eastern Australian coasts. Similar to Southern Free-tailed Bat but slightly higher frequency calls."),
        name: None,
    },
    // ── Uncommon ─────────────────────────────────────────────────
    BookEntryDef {
        species: &species::FALSISTRELLUS_TASMANIENSIS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Large vesper bat of south-eastern forests. Roosts in tree hollows. Vulnerable (IUCN). Distinctive mid-range frequency calls."),
        name: None,
    },
    BookEntryDef {
        species: &species::SCOTOREPENS_ORION,
        commonness: Some(Commonness::Uncommon),
        description: Some("Robust bat of south-eastern coastal forests. Narrow frequency range distinctive. Roosts in tree hollows."),
        name: None,
    },
    BookEntryDef {
        species: &species::SCOTOREPENS_BALSTONI,
        commonness: Some(Commonness::Uncommon),
        description: Some("Widespread across inland Australia. Found in drier regions of northern and western Victoria. Similar frequency to Gould's Wattled Bat."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_MACROPUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Australia's only fishing bat. Trawls water surfaces with large feet. Found near rivers, lakes, and dams. Very quiet calls."),
        name: None,
    },
    BookEntryDef {
        species: &species::SACCOLAIMUS_FLAVIVENTRIS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Large, fast-flying bat with glossy black fur and yellow belly. Migratory; visits Victoria seasonally. High-altitude forager."),
        name: None,
    },
    BookEntryDef {
        species: &species::RHINOLOPHUS_MEGAPHYLLUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Constant-frequency echolocation using distinctive horseshoe-shaped noseleaf. Cave-roosting. Found in forests of eastern and southern Victoria."),
        name: None,
    },
    // ── Rare ─────────────────────────────────────────────────────
    BookEntryDef {
        species: &species::NYCTOPHILUS_MAJOR,
        commonness: Some(Commonness::Rare),
        description: Some("Formerly N. timoriensis. Rare in Victoria, restricted to drier woodlands. Very quiet gleaning calls. Vulnerable."),
        name: None,
    },
    BookEntryDef {
        species: &species::VESPADELUS_BAVERSTOCKI,
        commonness: Some(Commonness::Rare),
        description: Some("Small bat of inland Australia. In Victoria, restricted to the semi-arid northwest (Mallee region)."),
        name: None,
    },
    BookEntryDef {
        species: &species::SCOTEANAX_RUEPPELLII,
        commonness: Some(Commonness::Rare),
        description: Some("Large, robust bat of eastern coastal forests. Rare in Victoria, mainly in far east Gippsland. Aggressive predator of large insects and small vertebrates."),
        name: None,
    },
    // ── Endangered ───────────────────────────────────────────────
    BookEntryDef {
        species: &species::MINIOPTERUS_ORIANAE_BASSANII,
        commonness: Some(Commonness::Endangered),
        description: Some("Critically Endangered (EPBC Act). Dependent on a single maternity cave near Warrnambool. Population <50 individuals. Southwest Victoria only."),
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTOPHILUS_CORBENI,
        commonness: Some(Commonness::Endangered),
        description: Some("Vulnerable (EPBC Act). Extremely rare in Victoria; restricted to northwest Mallee (Hattah-Kulkyne, Gunbower). Possibly <50 individuals in VIC."),
        name: None,
    },
    // ── Non-echolocating (sorted to end by get_manifest) ────────
    BookEntryDef {
        species: &species::PTEROPUS_POLIOCEPHALUS,
        commonness: Some(Commonness::Rare),
        description: Some("Australia's largest bat (wingspan ~1 m). Does not echolocate. Camps in colonies along waterways. Vulnerable (EPBC Act). Pollinator and seed disperser."),
        name: None,
    },
    BookEntryDef {
        species: &species::PTEROPUS_SCAPULATUS,
        commonness: Some(Commonness::Vagrant),
        description: Some("Seasonal visitor to northern Victoria. Does not echolocate. Nomadic, following eucalypt flowering. Occasionally camps at Swan Hill and Numurkah."),
        name: None,
    },
];

// ══════════════════════════════════════════════════════════════════════════════
// Europe — species-level entries sorted by commonness
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Dietz, Helversen & Nill (2009): Bats of Britain, Europe and Northwest Africa
// - Russ (2012): British Bat Calls: A Guide to Species Identification
// - Barataud (2015): Acoustic Ecology of European Bats

const EUROPE_BOOK: &[BookEntryDef] = &[
    // ── Very Common ──────────────────────────────────────────────
    BookEntryDef {
        species: &species::PIPISTRELLUS_PIPISTRELLUS,
        commonness: Some(Commonness::VeryCommon),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::PIPISTRELLUS_PYGMAEUS,
        commonness: Some(Commonness::VeryCommon),
        description: None,
        name: None,
    },
    // ── Common ───────────────────────────────────────────────────
    BookEntryDef {
        species: &species::PIPISTRELLUS_NATHUSII,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::PIPISTRELLUS_KUHLII,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_DAUBENTONII,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_NATTERERI,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_MYSTACINUS,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_BRANDTII,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_MYOTIS,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTALUS_NOCTULA,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTALUS_LEISLERI,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::EPTESICUS_SEROTINUS,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::EPTESICUS_NILSSONII,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::PLECOTUS_AURITUS,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::RHINOLOPHUS_FERRUMEQUINUM,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::RHINOLOPHUS_HIPPOSIDEROS,
        commonness: Some(Commonness::Common),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::MINIOPTERUS_SCHREIBERSII,
        commonness: Some(Commonness::Common),
        description: Some("Fast, agile cave-dweller found across southern Europe. Long, narrow wings. Formerly one species; now split into several. Sensitive to cave disturbance."),
        name: None,
    },
    // ── Uncommon ─────────────────────────────────────────────────
    BookEntryDef {
        species: &species::BARBASTELLA_BARBASTELLUS,
        commonness: Some(Commonness::Uncommon),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::PLECOTUS_AUSTRIACUS,
        commonness: Some(Commonness::Uncommon),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::VESPERTILIO_MURINUS,
        commonness: Some(Commonness::Uncommon),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_BECHSTEINII,
        commonness: Some(Commonness::Uncommon),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_DASYCNEME,
        commonness: Some(Commonness::Uncommon),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::TADARIDA_TENIOTIS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Europe's only free-tailed bat. Loud, low-frequency calls audible to humans. Fast, high-altitude forager. Restricted to the Mediterranean; roosts in cliff crevices and tall buildings."),
        name: None,
    },
    // ── Rare ─────────────────────────────────────────────────────
    BookEntryDef {
        species: &species::RHINOLOPHUS_EURYALE,
        commonness: Some(Commonness::Rare),
        description: None,
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTALUS_LASIOPTERUS,
        commonness: Some(Commonness::Rare),
        description: None,
        name: None,
    },
];

// ══════════════════════════════════════════════════════════════════════════════
// Costa Rica — species-level entries, tiered by commonness
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Jung et al. (2007): Echolocation calls in Central American emballonurids
// - Leiser-Miller & Santana (2021): Phyllostomid echolocation (Costa Rica data)
// - Gessinger et al. (2019): CF-FM echolocation of Lonchorhina aurita
// - Zamora-Gutierrez et al. (2016): Acoustic identification of Mexican bats
// - Rydell et al. (2002): Acoustic identification of Yucatan bats
//
// Phyllostomidae are low-intensity "whispering" echolocators, typically
// detectable only within a few meters. Descriptions note this limitation.

const COSTA_RICA_BOOK: &[BookEntryDef] = &[
    // ── Very Common ──────────────────────────────────────────────
    // Easily detected species with loud calls

    BookEntryDef {
        species: &species::SACCOPTERYX_BILINEATA,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Abundant in lowland forests. Roosts on tree trunks and building walls. Alternates ~45/48 kHz. Vocal learner with complex song repertoire."),
        name: None,
    },
    BookEntryDef {
        species: &species::MOLOSSUS_MOLOSSUS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Abundant in buildings and urban areas. Alternating QCF at ~34.5/39.6 kHz. One of the first species heard at dusk. Open-space aerial hawker."),
        name: None,
    },
    BookEntryDef {
        species: &species::PTERONOTUS_MESOAMERICANUS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Long CF at ~61 kHz with Doppler compensation\u{2014}the ONLY high-duty-cycle echolocator in the New World. Unmistakable call. Huge cave colonies."),
        name: None,
    },
    BookEntryDef {
        species: &species::CAROLLIA_PERSPICILLATA,
        commonness: Some(Commonness::VeryCommon),
        description: Some("One of the most abundant Neotropical bats. Peak ~71 kHz. Low-intensity whispering calls\u{2014}detectable only within a few meters. Key seed disperser."),
        name: None,
    },
    BookEntryDef {
        species: &species::ARTIBEUS_JAMAICENSIS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Very common frugivore. Peak ~56 kHz. Variable intensity; not always a quiet whisperer. Important fig seed disperser throughout lowland forests."),
        name: None,
    },
    BookEntryDef {
        species: &species::TADARIDA_BRASILIENSIS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Extremely flexible acoustics: QCF 49\u{2013}70 kHz in open space, drops to 25\u{2013}40 kHz near objects. Forms massive colonies. Fast, high-altitude forager."),
        name: None,
    },
    BookEntryDef {
        species: &species::GLOSSOPHAGA_SORICINA,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Abundant nectarivore. Peak ~80 kHz. Low-intensity calls\u{2014}hard to detect beyond a few meters. Uses echolocation to find flowers with acoustic reflectors."),
        name: None,
    },
    BookEntryDef {
        species: &species::DESMODUS_ROTUNDUS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Common near livestock. Peak ~55 kHz. Relatively long calls for a phyllostomid (~5.5 ms). Low-intensity. Obligate blood-feeder with infrared-sensing nose pits."),
        name: None,
    },

    // ── Common ───────────────────────────────────────────────────

    BookEntryDef {
        species: &species::RHYNCHONYCTERIS_NASO,
        commonness: Some(Commonness::Common),
        description: Some("Tiny bat roosting in lines along riverbanks. CF-FM with peak at ~47 kHz. Drops from ~100 to ~67 kHz during prey pursuit. Cryptic bark-like camouflage."),
        name: None,
    },
    BookEntryDef {
        species: &species::BALANTIOPTERYX_PLICATA,
        commonness: Some(Commonness::Common),
        description: Some("Open-area forager near caves and buildings. Long QCF (14\u{2013}20 ms) at ~43 kHz. Displays jamming avoidance in groups by shifting peak frequency."),
        name: None,
    },
    BookEntryDef {
        species: &species::PEROPTERYX_MACROTIS,
        commonness: Some(Commonness::Common),
        description: Some("Multiharmonic QCF at ~40 kHz (2nd harmonic). Found near caves and rock shelters. Distinctive musky odor."),
        name: None,
    },
    BookEntryDef {
        species: &species::PTERONOTUS_DAVYI,
        commonness: Some(Commonness::Common),
        description: Some("CF-FM at ~67 kHz with sweep to ~51 kHz. Wing membranes fused across back (naked-backed appearance). Cave-roosting; often with P. mesoamericanus."),
        name: None,
    },
    BookEntryDef {
        species: &species::PTERONOTUS_GYMNONOTUS,
        commonness: Some(Commonness::Common),
        description: Some("CF at ~54\u{2013}57 kHz. Largest Pteronotus. Similar to P. davyi but lower frequency. Cave-dwelling."),
        name: None,
    },
    BookEntryDef {
        species: &species::MORMOOPS_MEGALOPHYLLA,
        commonness: Some(Commonness::Common),
        description: Some("Bizarre leaf-chin face. Fundamental suppressed; 2nd harmonic at ~67 kHz dominates recordings. Large cave colonies. Ghost-like appearance in flight."),
        name: None,
    },
    BookEntryDef {
        species: &species::NOCTILIO_LEPORINUS,
        commonness: Some(Commonness::Common),
        description: Some("Large fishing bat. Long CF at 53\u{2013}56 kHz + FM sweep. Rakes water with large clawed feet to catch fish. Found along rivers, lakes, and coasts."),
        name: None,
    },
    BookEntryDef {
        species: &species::MOLOSSUS_SINALOAE,
        commonness: Some(Commonness::Common),
        description: Some("QCF at ~34 kHz. Shifts frequency up ~6 kHz in urban noise (Lombard effect). Larger than M. molossus. Open-space forager."),
        name: None,
    },
    BookEntryDef {
        species: &species::MOLOSSUS_RUFUS,
        commonness: Some(Commonness::Common),
        description: Some("Large molossid with low-frequency QCF at ~25\u{2013}26 kHz. Roosts in buildings and hollow trees. Fast, direct flight."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_NIGRICANS,
        commonness: Some(Commonness::Common),
        description: Some("Highly plastic calls: narrowband ~7 ms in open space; broadband FM in clutter. Peak ~50 kHz. Common in forests and urban edges."),
        name: None,
    },
    BookEntryDef {
        species: &species::ARTIBEUS_LITURATUS,
        commonness: Some(Commonness::Common),
        description: Some("Large frugivore. Lower peak (~52 kHz) than A. jamaicensis. Low-intensity. Prominent facial stripes. Important pollinator and seed disperser."),
        name: None,
    },
    BookEntryDef {
        species: &species::STURNIRA_LILIUM,
        commonness: Some(Commonness::Common),
        description: Some("Frugivore with well-documented peak at ~66.5 kHz. Low-intensity FM. Yellow shoulder epaulettes in males. Common in forest and edge habitats."),
        name: None,
    },
    BookEntryDef {
        species: &species::URODERMA_BILOBATUM,
        commonness: Some(Commonness::Common),
        description: Some("Tent-roosting frugivore. Bites leaf ribs to create tent roosts. Peak ~70 kHz. Low-intensity nasal FM\u{2014}hard to detect. Lowland forests."),
        name: None,
    },
    BookEntryDef {
        species: &species::CAROLLIA_CASTANEA,
        commonness: Some(Commonness::Common),
        description: Some("Higher peak (~78 kHz) than C. perspicillata. Low-intensity FM. Frugivore preferring understory fruits. Common in wet lowland forests."),
        name: None,
    },
    BookEntryDef {
        species: &species::CAROLLIA_BREVICAUDA,
        commonness: Some(Commonness::Common),
        description: Some("Intermediate peak (~73 kHz) between C. perspicillata and C. castanea. Low-intensity FM. Frugivore. Premontane and montane forests."),
        name: None,
    },
    BookEntryDef {
        species: &species::GLOSSOPHAGA_COMMISSARISI,
        commonness: Some(Commonness::Common),
        description: Some("Nectarivore. Slightly lower peak (~75 kHz) than G. soricina. Low-intensity FM. Important pollinator of many tropical plants."),
        name: None,
    },
    BookEntryDef {
        species: &species::TRACHOPS_CIRRHOSUS,
        commonness: Some(Commonness::Common),
        description: Some("Famous frog-eating bat. Locates prey by listening to mating calls. Peak ~70 kHz. Low-intensity FM\u{2014}hard to detect. Warty lips for gripping frogs."),
        name: None,
    },
    BookEntryDef {
        species: &species::PHYLLOSTOMUS_HASTATUS,
        commonness: Some(Commonness::Common),
        description: Some("Large omnivore. One of the lowest-frequency phyllostomids (~47 kHz peak). Low-intensity FM. Harem groups in caves and hollow trees."),
        name: None,
    },
    BookEntryDef {
        species: &species::DERMANURA_PHAEOTIS,
        commonness: Some(Commonness::Common),
        description: Some("Small frugivore. Peak ~75 kHz. Low-intensity FM\u{2014}detectable only within a few meters. Common in lowland and premontane forests."),
        name: None,
    },
    BookEntryDef {
        species: &species::MICRONYCTERIS_MICROTIS,
        commonness: Some(Commonness::Common),
        description: Some("Gleaning insectivore. Very short broadband FM (0.3\u{2013}1 ms) at ~90\u{2013}100 kHz. Ultra-low intensity\u{2014}barely detectable beyond 2\u{2013}3 m. Can find motionless prey."),
        name: None,
    },
    BookEntryDef {
        species: &species::EPTESICUS_BRASILIENSIS,
        commonness: Some(Commonness::Common),
        description: Some("Peak ~54\u{2013}60 kHz. Source level ~101\u{2013}106 dB SPL. FM-QCF. Frequency varies with temperature. Forest edges and open areas."),
        name: None,
    },

    // ── Uncommon ─────────────────────────────────────────────────

    BookEntryDef {
        species: &species::SACCOPTERYX_LEPTURA,
        commonness: Some(Commonness::Uncommon),
        description: Some("Higher frequency (~50 kHz) than S. bilineata. Similar QCF structure. Thinner dorsal stripes. Less common; found in lowland forests."),
        name: None,
    },
    BookEntryDef {
        species: &species::CORMURA_BREVIROSTRIS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Unusual: most energy in 5th harmonic at ~68 kHz. Forest-interior forager. Multiharmonic calls."),
        name: None,
    },
    BookEntryDef {
        species: &species::PEROPTERYX_KAPPLERI,
        commonness: Some(Commonness::Uncommon),
        description: Some("Lower frequency (~32 kHz) than P. macrotis. 2nd harmonic dominant. Near caves and rocky outcrops in forested areas."),
        name: None,
    },
    BookEntryDef {
        species: &species::PTERONOTUS_PERSONATUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Highest frequency Pteronotus: initial CF ~83 kHz, terminal ~68 kHz. Doppler-shift compensation. Cave-roosting."),
        name: None,
    },
    BookEntryDef {
        species: &species::NOCTILIO_ALBIVENTRIS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Higher CF (~75 kHz) than N. leporinus. Trawls insects and small fish from water. Less common than greater bulldog bat."),
        name: None,
    },
    BookEntryDef {
        species: &species::MOLOSSUS_BONDAE,
        commonness: Some(Commonness::Uncommon),
        description: Some("QCF at ~33 kHz. Open-space forager. Roosts in buildings. Slightly lower frequency than M. molossus."),
        name: None,
    },
    BookEntryDef {
        species: &species::MOLOSSUS_COIBENSIS,
        commonness: Some(Commonness::Uncommon),
        description: Some("QCF at ~35 kHz. Originally described from Coiba Island, Panama. Open-space forager. Smaller than other Molossus species."),
        name: None,
    },
    BookEntryDef {
        species: &species::EUMOPS_AURIPENDULUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Large molossid. Alternating QCF at ~23\u{2013}26 kHz. High, fast flight above canopy. Long-duration narrowband calls."),
        name: None,
    },
    BookEntryDef {
        species: &species::EUMOPS_GLAUCINUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Very low frequency (~22\u{2013}25 kHz) QCF. Large bat with long narrow wings. High-altitude forager above canopy."),
        name: None,
    },
    BookEntryDef {
        species: &species::CYNOMOPS_GREENHALLI,
        commonness: Some(Commonness::Uncommon),
        description: Some("Low frequency (~22 kHz) open-space forager. Flat face with forward-pointing nostrils. Roosts in buildings and hollow trees."),
        name: None,
    },
    BookEntryDef {
        species: &species::PROMOPS_CENTRALIS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Distinctive upward-modulated QCF (unusual for molossids). Alternating pairs at ~30/35 kHz. Easily recognized on bat detector."),
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTINOMOPS_LATICAUDATUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Three-frequency alternation (~26.7, 28.7, 32.4 kHz). Open-space forager. Roosts in rock crevices and buildings."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_KEAYSI,
        commonness: Some(Commonness::Uncommon),
        description: Some("High repetition rates (15\u{2013}20/s). Short FM calls (~2.5 ms). Peak ~55 kHz. Found in highlands and cloud forests."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_RIPARIUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Steep broadband FM sweep from ~120 to ~50 kHz. Short calls (~2 ms). Forages near streams and forest edges. Recorded in Costa Rica."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_ELEGANS,
        commonness: Some(Commonness::Uncommon),
        description: Some("High-frequency FM (~55 kHz peak). Difficult to distinguish from M. nigricans acoustically. Small Myotis of lowland forests."),
        name: None,
    },
    BookEntryDef {
        species: &species::EPTESICUS_FURINALIS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Lower frequency (~43 kHz) than E. brasiliensis. FM-QCF. More FM in cluttered habitats. Forest edges."),
        name: None,
    },
    BookEntryDef {
        species: &species::LASIURUS_BLOSSEVILLII,
        commonness: Some(Commonness::Uncommon),
        description: Some("Open-air forager. Peak ~42 kHz. FM-QCF. Migratory. Roosts solitarily in foliage. Distinctive reddish fur."),
        name: None,
    },
    BookEntryDef {
        species: &species::LASIURUS_EGA,
        commonness: Some(Commonness::Uncommon),
        description: Some("Lower peak (~35 kHz) than L. blossevillii. FM-QCF. Roosts in palm fronds. Open-air forager around street lights."),
        name: None,
    },
    BookEntryDef {
        species: &species::RHOGEESSA_TUMIDA,
        commonness: Some(Commonness::Uncommon),
        description: Some("Small vespertilionid. Broadband FM + QCF termination at ~48 kHz. Forages low in forest gaps and edges."),
        name: None,
    },
    BookEntryDef {
        species: &species::LONCHORHINA_AURITA,
        commonness: Some(Commonness::Uncommon),
        description: Some("UNIQUE phyllostomid with CF-FM calls. Long CF at ~45 kHz (3rd harmonic). Longest phyllostomid calls (up to 8.7 ms). Extremely long nose-leaf. Cave-roosting."),
        name: None,
    },
    BookEntryDef {
        species: &species::PHYLLOSTOMUS_DISCOLOR,
        commonness: Some(Commonness::Uncommon),
        description: Some("Omnivore. Peak ~55 kHz. Low-intensity FM. Large colonies in hollow trees. Best hearing at 20 kHz. Low-frequency for a phyllostomid."),
        name: None,
    },
    BookEntryDef {
        species: &species::LOPHOSTOMA_SILVICOLUM,
        commonness: Some(Commonness::Uncommon),
        description: Some("Gleaning insectivore that modifies termite nests into roosts. Peak ~70 kHz. Low-intensity FM\u{2014}detectable only within a few meters."),
        name: None,
    },
    BookEntryDef {
        species: &species::ANOURA_GEOFFROYI,
        commonness: Some(Commonness::Uncommon),
        description: Some("High-altitude nectarivore. Peak ~70 kHz. Low-intensity FM. Cloud forests and highlands. Important pollinator."),
        name: None,
    },
    BookEntryDef {
        species: &species::CENTURIO_SENEX,
        commonness: Some(Commonness::Uncommon),
        description: Some("Bizarre wrinkled face with retractable skin mask. Peak ~65 kHz. Relatively long calls for a stenodermatine (1\u{2013}3 ms). Frugivore. Low-intensity."),
        name: None,
    },

    // ── Rare ─────────────────────────────────────────────────────

    BookEntryDef {
        species: &species::DICLIDURUS_ALBUS,
        commonness: Some(Commonness::Rare),
        description: Some("Distinctive white fur. Narrowband QCF at ~24 kHz. Rarely encountered. High-altitude open-space forager. One of the most striking-looking bats."),
        name: None,
    },
    BookEntryDef {
        species: &species::EPTESICUS_FUSCUS,
        commonness: Some(Commonness::Rare),
        description: Some("Large vespertilionid. Peak ~30 kHz. FM-QCF. At southern edge of range in Costa Rica. Uncommon in highlands."),
        name: None,
    },
    BookEntryDef {
        species: &species::VAMPYRUM_SPECTRUM,
        commonness: Some(Commonness::Rare),
        description: Some("Largest bat in the Americas (wingspan ~1 m). Peak ~70 kHz. Low-intensity FM\u{2014}very difficult to detect acoustically. Carnivorous: preys on birds and other bats."),
        name: None,
    },
    BookEntryDef {
        species: &species::CHROTOPTERUS_AURITUS,
        commonness: Some(Commonness::Rare),
        description: Some("Carnivorous gleaner. Peak ~77 kHz. Short FM (0.8\u{2013}1.4 ms). Low-intensity\u{2014}hard to detect beyond a few meters. Large ears; hunts other bats and rodents."),
        name: None,
    },
    BookEntryDef {
        species: &species::MACROPHYLLUM_MACROPHYLLUM,
        commonness: Some(Commonness::Rare),
        description: Some("Unusual trawling phyllostomid. Louder than most relatives (~101 dB SPL). Peak ~85 kHz. Large feet for grabbing insects from water surfaces."),
        name: None,
    },
    BookEntryDef {
        species: &species::MICRONYCTERIS_HIRSUTA,
        commonness: Some(Commonness::Rare),
        description: Some("Gleaning insectivore. Lower peak (~52 kHz) than M. microtis. Low-intensity FM. Documented from Costa Rica. Forest interior."),
        name: None,
    },
    BookEntryDef {
        species: &species::MIMON_CRENULATUM,
        commonness: Some(Commonness::Rare),
        description: Some("Gleaning insectivore. Peak ~75 kHz. Low-intensity FM. Now Gardnerycteris crenulatum. Forest understory."),
        name: None,
    },
    BookEntryDef {
        species: &species::TONATIA_SAUROPHILA,
        commonness: Some(Commonness::Rare),
        description: Some("Gleaning insectivore/carnivore. Peak ~65 kHz. Low-intensity FM. Forest interior specialist. Roosts in hollow trees."),
        name: None,
    },
    BookEntryDef {
        species: &species::LAMPRONYCTERIS_BRACHYOTIS,
        commonness: Some(Commonness::Rare),
        description: Some("Rare gleaning insectivore. Peak ~75 kHz. Low-intensity FM. Poorly documented acoustically. Yellow throat patches."),
        name: None,
    },
    BookEntryDef {
        species: &species::GLYPHONYCTERIS_SYLVESTRIS,
        commonness: Some(Commonness::Rare),
        description: Some("Rare gleaner. Peak ~85 kHz. Ultra-short broadband FM (0.3\u{2013}1 ms). Very low intensity. Forest interior."),
        name: None,
    },
    BookEntryDef {
        species: &species::TRINYCTERIS_NICEFORI,
        commonness: Some(Commonness::Rare),
        description: Some("Low-intensity gleaner. Peak ~80 kHz. Multiharmonic FM. Forest understory specialist. Rarely captured or detected."),
        name: None,
    },
    BookEntryDef {
        species: &species::HYLONYCTERIS_UNDERWOODI,
        commonness: Some(Commonness::Rare),
        description: Some("Very small nectarivore. High frequency peak ~90 kHz. Low-intensity FM. Montane cloud forests. Poorly known acoustically."),
        name: None,
    },
    BookEntryDef {
        species: &species::MESOPHYLLA_MACCONNELLI,
        commonness: Some(Commonness::Rare),
        description: Some("Tiny (5\u{2013}7 g) with the highest peak frequency of any phyllostomid (~100\u{2013}118 kHz). Ultra-low intensity. Tent-roosting frugivore."),
        name: None,
    },
    BookEntryDef {
        species: &species::ECTOPHYLLA_ALBA,
        commonness: Some(Commonness::Rare),
        description: Some("Iconic tiny white bat. Peak ~75 kHz. Low-intensity FM. Roosts in Heliconia leaf tents. Endemic to Central America. Specializes on one fig species."),
        name: None,
    },
    BookEntryDef {
        species: &species::THYROPTERA_TRICOLOR,
        commonness: Some(Commonness::Rare),
        description: Some("Suction-cup disks for roosting in rolled Heliconia leaves. Extremely low intensity\u{2014}barely detectable at <1 m. Distinctive social calls for roost-finding."),
        name: None,
    },
    BookEntryDef {
        species: &species::NATALUS_MEXICANUS,
        commonness: Some(Commonness::Rare),
        description: Some("Among the highest frequency bats: peak ~100\u{2013}130 kHz. Very low intensity\u{2014}barely detectable beyond 50 cm. Delicate, cave-roosting. Formerly N. stramineus."),
        name: None,
    },
    BookEntryDef {
        species: &species::BAUERUS_DUBIAQUERCUS,
        commonness: Some(Commonness::Rare),
        description: Some("Very quiet calls (~35 kHz peak). Plecotus-like gleaning insectivore. Rare and poorly known. Montane forests."),
        name: None,
    },
];

// ══════════════════════════════════════════════════════════════════════════════
// Japan — species-level entries sorted by commonness
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Fukui et al. (2004) Zool Sci: Acoustic ID of eight Hokkaido species
// - Funakoshi & Uchida (1978): R. ferrumequinum nippon CF frequency
// - Hiryu et al. (2006): R. pumilus geographic variation on Okinawa
// - Fujioka et al. (2014): CNN bat species ID system for 30 Japanese species
// - IUCN Red List; Ministry of the Environment (Japan) Red Data Book

const JAPAN_BOOK: &[BookEntryDef] = &[
    // ── Very Common ──────────────────────────────────────────────
    BookEntryDef {
        species: &species::PIPISTRELLUS_ABRAMUS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Japan's most common urban bat. FM sweeps ~80\u{2013}95 kHz to terminal ~40 kHz, peak ~52 kHz. Roosts in buildings, forages around streetlights. Found throughout the archipelago."),
        name: None,
    },

    // ── Common ───────────────────────────────────────────────────
    BookEntryDef {
        species: &species::RHINOLOPHUS_FERRUMEQUINUM_NIPPON,
        commonness: Some(Commonness::Common),
        description: Some("CF-FM calls with diagnostic CF2 at ~65\u{2013}69 kHz. FM/CF/FM structure. Widespread from Hokkaido to Kyushu. Cave, mine, and tunnel roosts. Key species for Doppler-shift research."),
        name: None,
    },
    BookEntryDef {
        species: &species::RHINOLOPHUS_CORNUTUS,
        commonness: Some(Commonness::Common),
        description: Some("CF-FM calls with peak CF ~103\u{2013}111 kHz, increasing from north to south (103\u{2013}104 kHz in Aomori, 108\u{2013}111 kHz on Amami-Oshima). Endemic to Japan. Caves and buildings."),
        name: None,
    },
    BookEntryDef {
        species: &species::MINIOPTERUS_FULIGINOSUS,
        commonness: Some(Commonness::Common),
        description: Some("FM downward sweeps, terminal frequency ~44\u{2013}50 kHz. Shows geographic variation in peak frequency among colonies. Fast agile flier. Large cave maternity colonies. Honshu to Ryukyus."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_MACRODACTYLUS,
        commonness: Some(Commonness::Common),
        description: Some("Steep FM sweeps ~90\u{2013}40 kHz, peak ~50 kHz. Trawling insectivore using large feet to gaff prey from water surfaces. Rivers and streams throughout Japan."),
        name: None,
    },
    BookEntryDef {
        species: &species::VESPERTILIO_SINENSIS,
        commonness: Some(Commonness::Common),
        description: Some("FM-QCF calls, peak ~24 kHz, max ~46 kHz. Steep FM into shallow QCF tail. Migratory species with seasonal movements through the archipelago. Hokkaido to Kyushu."),
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTALUS_AVIATOR,
        commonness: Some(Commonness::Common),
        description: Some("FM-QCF calls, peak ~21 kHz, duration ~12 ms\u{2014}longest among Hokkaido bats. Japan's largest insectivorous bat. Forages up to 300 m altitude. Occasionally preys on migrating birds."),
        name: None,
    },
    BookEntryDef {
        species: &species::PLECOTUS_SACRIMONTIS,
        commonness: Some(Commonness::Common),
        description: Some("Low-intensity FM calls, start ~46 kHz, peak ~41 kHz, end ~23 kHz. Gleaning forager specialising in fluttering moths via passive listening. Endemic to Japan. Forest-dwelling."),
        name: None,
    },
    BookEntryDef {
        species: &species::MURINA_HILGENDORFI,
        commonness: Some(Commonness::Common),
        description: Some("Ultra-broadband FM sweeps ~145\u{2013}165 kHz down to ~45\u{2013}55 kHz. Forest gleaner. Hokkaido to Kyushu. Roosts in tree hollows and dead curled leaves."),
        name: None,
    },
    BookEntryDef {
        species: &species::MURINA_USSURIENSIS,
        commonness: Some(Commonness::Common),
        description: Some("Ultra-broadband FM sweeps similar to M. hilgendorfi. Tiny bat (4\u{2013}8 g). Remarkably hibernates under snow. Roosts in curled dead leaves. Hokkaido and Honshu."),
        name: None,
    },
    BookEntryDef {
        species: &species::MURINA_SILVATICA,
        commonness: Some(Commonness::Common),
        description: Some("Broadband FM calls typical of Murina. Distribution spans ~2000 km north\u{2013}south across Japan. Sometimes considered conspecific with M. ussuriensis. Curled-leaf roosts."),
        name: None,
    },

    // ── Uncommon ─────────────────────────────────────────────────
    BookEntryDef {
        species: &species::RHINOLOPHUS_PUMILUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("CF-FM calls with CF ~80\u{2013}90 kHz. Shows 5\u{2013}8 kHz dialect difference between northern and southern Okinawa populations, maintained by maternal transmission. Central and southern Ryukyus."),
        name: None,
    },
    BookEntryDef {
        species: &species::RHINOLOPHUS_IMAIZUMII,
        commonness: Some(Commonness::Uncommon),
        description: Some("CF-FM calls at frequency intermediate between R. cornutus and R. pumilus (~94\u{2013}108 kHz). Temperate forests on Honshu and Shikoku. Taxonomic status debated."),
        name: None,
    },
    BookEntryDef {
        species: &species::MINIOPTERUS_FUSCUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("FM calls slightly higher frequency than M. fuliginosus due to smaller body size. Ryukyu Islands and southern Kyushu. Cave-roosting."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_IKONNIKOVI,
        commonness: Some(Commonness::Uncommon),
        description: Some("Steep FM sweeps, peak ~50.6 kHz, start ~90 kHz, end ~43 kHz, duration ~2 ms. Small forest bat of Hokkaido and northern Honshu."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_BOMBINUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Very broadband FM calls sweeping from ~150 kHz down to ~20 kHz. Gleaning insectivore, hawks near vegetation. Forests of Kyushu and other regions."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_FRATER,
        commonness: Some(Commonness::Uncommon),
        description: Some("Brief FM pulses ~110\u{2013}50 kHz, duration ~3.5 ms. Edge-space forager near cliffs and caves. Honshu and Kyushu."),
        name: None,
    },
    BookEntryDef {
        species: &species::EPTESICUS_NILSSONII_JP,
        commonness: Some(Commonness::Uncommon),
        description: Some("FM-QCF calls, peak ~30.5 kHz, duration ~6 ms, bandwidth ~32 kHz. Hokkaido and northern Honshu. The most northerly-ranging bat in the world."),
        name: None,
    },

    // ── Rare ─────────────────────────────────────────────────────
    BookEntryDef {
        species: &species::RHINOLOPHUS_PERDITUS,
        commonness: Some(Commonness::Rare),
        description: Some("CF-FM calls, peak CF ~92\u{2013}98 kHz (92\u{2013}93 on Iriomote, 96\u{2013}98 on Ishigaki). Endemic to the Yaeyama Islands. Forest-dwelling, cave-roosting."),
        name: None,
    },
    BookEntryDef {
        species: &species::HIPPOSIDEROS_TURPIS,
        commonness: Some(Commonness::Rare),
        description: Some("CF calls typical of hipposiderids, ~65\u{2013}80 kHz. Southern Ryukyu Islands (Ishigaki, Iriomote, Miyako). Limestone cave roosts. Feeds mainly on beetles."),
        name: None,
    },
    BookEntryDef {
        species: &species::TAPHOZOUS_MELANOPOGON,
        commonness: Some(Commonness::Rare),
        description: Some("Low-intensity FM calls, dominant ~29\u{2013}33 kHz with four harmonics. Marginal occurrence in Japan (Ryukyu Islands). Roosts on rock walls and in caves."),
        name: None,
    },
    BookEntryDef {
        species: &species::TADARIDA_INSIGNIS,
        commonness: Some(Commonness::Rare),
        description: Some("Low-frequency FM-QCF calls ~18\u{2013}25 kHz. Fast, high-flying open-air forager. Western Japan. Roosts in rock crevices and buildings."),
        name: None,
    },
    BookEntryDef {
        species: &species::TADARIDA_LATOUCHEI,
        commonness: Some(Commonness::Rare),
        description: Some("Echolocation ~20 kHz. High-altitude flier, very difficult to capture. Known in Japan from a single specimen on Amami-Oshima (1985). IUCN Data Deficient."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_PRUINOSUS,
        commonness: Some(Commonness::Rare),
        description: Some("FM sweeps typical of Myotis. Endemic to Japan (Honshu, Shikoku, Kyushu). Forest-dwelling, roosts in tree hollows and buildings. Named for frosted fur."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_FORMOSUS,
        commonness: Some(Commonness::Rare),
        description: Some("Steep downward FM calls. Distinctive orange-brown coloring. Tsushima Island and western Japan. Forest-dwelling insectivore."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_HOSONOI,
        commonness: Some(Commonness::Rare),
        description: Some("FM calls typical of Myotis. Endemic to Japan (Honshu). Cave-dwelling. Poorly studied species."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_GRACILIS,
        commonness: Some(Commonness::Rare),
        description: Some("FM calls typical of small Myotis. Endemic to Japan (Honshu). Forest-dwelling. Limited published acoustic data."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_OZENSIS,
        commonness: Some(Commonness::Rare),
        description: Some("FM calls typical of Myotis. Endemic to central Honshu mountains. Cave-dwelling. Very limited distribution."),
        name: None,
    },
    BookEntryDef {
        species: &species::PIPISTRELLUS_ENDOI,
        commonness: Some(Commonness::Rare),
        description: Some("FM-QCF calls similar to P. abramus but slightly lower frequency. Endemic to Japan (Honshu). Temperate forests at 100\u{2013}1500 m. IUCN Near Threatened."),
        name: None,
    },
    BookEntryDef {
        species: &species::EPTESICUS_JAPONENSIS,
        commonness: Some(Commonness::Rare),
        description: Some("FM-QCF calls similar to E. nilssonii. Endemic to Japan (Honshu, Shikoku, Kyushu). Forest-dwelling, roosts in tree hollows and buildings."),
        name: None,
    },
    BookEntryDef {
        species: &species::BARBASTELLA_LEUCOMELAS,
        commonness: Some(Commonness::Rare),
        description: Some("FM calls in two alternating types: type A at 32\u{2013}37 kHz, type B at 38\u{2013}45 kHz. Very elusive. Rare in Japan (Honshu). Forest-dwelling."),
        name: None,
    },
    BookEntryDef {
        species: &species::MURINA_RYUKYUANA,
        commonness: Some(Commonness::Rare),
        description: Some("Broadband FM calls typical of Murina. Endemic to the Ryukyu Islands. Recently described species from Okinawa. Forest-dwelling."),
        name: None,
    },

    // ── Endangered ───────────────────────────────────────────────
    BookEntryDef {
        species: &species::MYOTIS_YANBARENSIS,
        commonness: Some(Commonness::Endangered),
        description: Some("FM calls above 40 kHz. Endemic to northern Okinawa (Yanbaru forest). Described in 1997. One of the rarest bats in Japan. Subtropical forest only."),
        name: None,
    },
    BookEntryDef {
        species: &species::MURINA_TENEBROSA,
        commonness: Some(Commonness::Endangered),
        description: Some("FM calls presumed similar to other Murina. Known only from a single holotype on Tsushima Island (1962). Possibly extinct due to deforestation. Alliance for Zero Extinction species."),
        name: None,
    },
    BookEntryDef {
        species: &species::PIPISTRELLUS_STURDEEI,
        commonness: Some(Commonness::Endangered),
        description: Some("Presumed FM-QCF calls. Known only from a specimen on the Bonin Islands (Ogasawara). Not seen since 1915\u{2014}possibly extinct."),
        name: None,
    },

    // ── Non-echolocating ─────────────────────────────────────────
    BookEntryDef {
        species: &species::PTEROPUS_DASYMALLUS,
        commonness: Some(Commonness::Endangered),
        description: Some("Large fruit bat of the Ryukyu Islands (4 subspecies in Japan). Does not echolocate. Feeds on fruit, flowers, and leaves. Endangered due to habitat loss and hunting."),
        name: None,
    },
];

// ══════════════════════════════════════════════════════════════════════════════
// United Kingdom — all 18 resident species + vagrants
// ══════════════════════════════════════════════════════════════════════════════
//
// Sources:
// - Russ (2012): British Bat Calls — A Guide to Species Identification
// - Dietz & Kiefer (2016): Bats of Britain and Europe
// - BCT (Bat Conservation Trust): UK bat species profiles
// - Barlow & Jones (1999): Pipistrellus pipistrellus / pygmaeus cryptic species
// - Jan et al. (2010): First UK record of Myotis alcathoe

const UK_BOOK: &[BookEntryDef] = &[
    // ── Very Common ──────────────────────────────────────────────
    BookEntryDef {
        species: &species::PIPISTRELLUS_PIPISTRELLUS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Britain's most common bat. Peak frequency ~45 kHz; calls heard on almost every bat detector outing. Roosts in buildings, forages along hedgerows, woodland edges, and over water. Often the first species new bat workers learn to identify."),
        name: None,
    },
    BookEntryDef {
        species: &species::PIPISTRELLUS_PYGMAEUS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Split from common pipistrelle in 1999. Peak frequency ~55 kHz. Strongly associated with waterways and riparian habitats across the UK. Maternity roosts can hold over 1,000 individuals. Sometimes called the 55 kHz pipistrelle."),
        name: None,
    },
    BookEntryDef {
        species: &species::PLECOTUS_AURITUS,
        commonness: Some(Commonness::VeryCommon),
        description: Some("Widespread across Britain. Very quiet, broadband FM calls (25–50 kHz) rarely detected beyond a few metres. Gleaning feeder — picks moths and spiders from foliage. Roosts in older buildings, churches, and tree holes. Ears nearly as long as the body."),
        name: None,
    },
    // ── Common ───────────────────────────────────────────────────
    BookEntryDef {
        species: &species::MYOTIS_DAUBENTONII,
        commonness: Some(Commonness::Common),
        description: Some("The 'water bat' — iconic low passes skimming rivers, canals, and lakes, scooping insects from the surface. Regular FM calls sweep from ~85 kHz to ~32 kHz. Roosts under bridges and in tunnels. Widespread across England, Wales, and Scotland."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_NATTERERI,
        commonness: Some(Commonness::Common),
        description: Some("Broadband FM sweeps from ~115 kHz down to ~25 kHz. Gleaner — hunts close to vegetation, sometimes hovering to pick prey off leaves. Distinctive fringe of stiff hairs along the tail membrane. Roosts in old buildings, trees, and bat boxes across Britain."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_MYSTACINUS,
        commonness: Some(Commonness::Common),
        description: Some("Small Myotis, widespread but under-recorded. FM calls sweep ~90–35 kHz, very similar to Brandt's bat on a detector. Reliable separation requires examination in the hand. Roosts in buildings; forages along woodland edges and over water."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_BRANDTII,
        commonness: Some(Commonness::Common),
        description: Some("Cryptic species pair with whiskered bat — only separated in 1970. Calls nearly identical (~90–35 kHz FM). Tends to be more woodland-associated than whiskered. Found across England and Wales; scarcer in Scotland."),
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTALUS_NOCTULA,
        commonness: Some(Commonness::Common),
        description: Some("Britain's largest common bat. Distinctive loud, narrow-band calls around 20–25 kHz, often alternating with steep FM sweeps. One of the first bats out at dusk — sometimes flies in daylight. Open-air forager, often high above the canopy. Roosts in tree holes."),
        name: None,
    },
    BookEntryDef {
        species: &species::NYCTALUS_LEISLERI,
        commonness: Some(Commonness::Common),
        description: Some("Smaller cousin of the noctule. Calls at ~25–27 kHz — slightly higher than noctule. Fast, high-flying forager. More common in Ireland than in Great Britain. Roosts in tree holes and bat boxes; rarely in buildings."),
        name: None,
    },
    // ── Uncommon ─────────────────────────────────────────────────
    BookEntryDef {
        species: &species::EPTESICUS_SEROTINUS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Large bat of southern England. Loud FM-QCF calls with characteristic frequency around 25–28 kHz. Slow, heavy flight often along treelines and around street lights. Roosts almost exclusively in buildings. Range rarely extends north of the Midlands."),
        name: None,
    },
    BookEntryDef {
        species: &species::PIPISTRELLUS_NATHUSII,
        commonness: Some(Commonness::Uncommon),
        description: Some("Long-distance migrant — birds ringed in Latvia have been found in the UK. Peak frequency ~38 kHz; calls can resemble common pip but slightly lower and often with distinctive social calls. Breeding colonies now established in several UK sites. Associated with waterside habitats."),
        name: None,
    },
    BookEntryDef {
        species: &species::RHINOLOPHUS_FERRUMEQUINUM,
        commonness: Some(Commonness::Uncommon),
        description: Some("Britain's largest horseshoe bat. Constant-frequency calls at ~82 kHz — unmistakable warbling tone on a heterodyne detector. Restricted to south-west England and south Wales. Hibernates in caves and mines; summer roosts in large roof spaces. UK population internationally important."),
        name: None,
    },
    BookEntryDef {
        species: &species::RHINOLOPHUS_HIPPOSIDEROS,
        commonness: Some(Commonness::Uncommon),
        description: Some("Tiny horseshoe bat weighing only 5–9 g. CF calls at ~110 kHz. Found in western Britain — south-west England, Wales, and western Ireland. Very sensitive to roost disturbance and light pollution. Hibernates underground; maternity roosts in buildings."),
        name: None,
    },
    // ── Rare ─────────────────────────────────────────────────────
    BookEntryDef {
        species: &species::BARBASTELLA_BARBASTELLUS,
        commonness: Some(Commonness::Rare),
        description: Some("Distinctive flat-faced bat with alternating call frequencies — short FM pulses alternating between ~32 kHz and ~43 kHz. One of the UK's rarest bats. Hunts tympanate moths and can switch frequencies to evade moth hearing. Found in mature woodland in southern England; roosts behind loose bark."),
        name: None,
    },
    BookEntryDef {
        species: &species::PLECOTUS_AUSTRIACUS,
        commonness: Some(Commonness::Rare),
        description: Some("Very similar to brown long-eared but restricted to the southern coast of England, mainly around the Channel Islands border. Slightly louder calls than brown long-eared. Fewer than 1,000 individuals estimated in the UK. Roosts in buildings."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_BECHSTEINII,
        commonness: Some(Commonness::Rare),
        description: Some("Elusive woodland specialist of southern England. Long ears (for a Myotis) and broadband FM calls. Rarely caught in mist nets — stays within the canopy. Associated with ancient woodland with veteran trees. One of the UK's rarest resident bats."),
        name: None,
    },
    BookEntryDef {
        species: &species::MYOTIS_ALCATHOE,
        commonness: Some(Commonness::Rare),
        description: Some("Europe's smallest Myotis, confirmed in the UK in 2010. FM calls sweep from ~100 kHz to ~40 kHz with peak energy ~50–55 kHz — slightly higher than whiskered or Brandt's. Very few confirmed UK sites; likely under-recorded. Requires DNA or detailed morphometrics for reliable identification."),
        name: None,
    },
    // ── Endangered ───────────────────────────────────────────────
    BookEntryDef {
        species: &species::MYOTIS_MYOTIS,
        commonness: Some(Commonness::Endangered),
        description: Some("Britain's largest Myotis. Declared extinct as a UK breeding species in 1990, but a single male has hibernated in a Sussex mine since at least 2002. Loud FM calls sweep ~25–80 kHz. Gleaner — picks large beetles from the ground. Any UK sighting is nationally significant."),
        name: None,
    },
    // ── Vagrant ──────────────────────────────────────────────────
    BookEntryDef {
        species: &species::VESPERTILIO_MURINUS,
        commonness: Some(Commonness::Vagrant),
        description: Some("Continental migrant occasionally reaching eastern England and North Sea oil rigs, mostly in autumn. Distinctive alternating call pattern at ~24 kHz and ~30 kHz. Loud social calls audible to the human ear."),
        name: None,
    },
    BookEntryDef {
        species: &species::PIPISTRELLUS_KUHLII,
        commonness: Some(Commonness::Vagrant),
        description: Some("Mediterranean species expanding northward through Europe. Peak frequency ~40 kHz. Extremely rare in the UK with only a handful of confirmed records, but range expansion may bring more sightings."),
        name: None,
    },
    BookEntryDef {
        species: &species::HYPSUGO_SAVII,
        commonness: Some(Commonness::Vagrant),
        description: Some("Shallow FM sweep with quasi-constant-frequency tail at ~32–34 kHz. Primarily a Mediterranean species but increasingly recorded crossing the Channel to southern England. Distinctive call shape helps separate it from pipistrelles on a spectrogram."),
        name: None,
    },
    BookEntryDef {
        species: &species::EPTESICUS_NILSSONII,
        commonness: Some(Commonness::Vagrant),
        description: Some("Northern bat of Scandinavia and continental Europe. Calls around 27–30 kHz. Very rare UK vagrant — most records are from Scotland and the Northern Isles."),
        name: None,
    },
];

/// Get the bat book manifest for a given region.
///
/// Non-echolocating species are always sorted to the end (stable sort preserves
/// relative order within each group).
pub fn get_manifest(region: BatBookRegion) -> BatBookManifest {
    let book: &[BookEntryDef] = match region {
        BatBookRegion::VicAustralia => VIC_AUSTRALIA_BOOK,
        BatBookRegion::Europe => EUROPE_BOOK,
        BatBookRegion::CostaRica => COSTA_RICA_BOOK,
        BatBookRegion::Japan => JAPAN_BOOK,
        BatBookRegion::UK => UK_BOOK,
        _ => GLOBAL_BOOK,
    };
    let mut entries: Vec<_> = book.iter().map(|e| e.materialize()).collect();
    // Stable sort: echolocating first, non-echolocating last
    entries.sort_by_key(|e| if e.echolocates { 0u8 } else { 1 });
    BatBookManifest {
        region: region.short_label().to_string(),
        entries,
    }
}
