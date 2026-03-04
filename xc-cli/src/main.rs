use clap::{Parser, Subcommand};
use std::path::PathBuf;
use xc_lib::{api, cache, key_store, taxonomy, XC_GROUPS};

#[derive(Parser)]
#[command(name = "xc-fetch", about = "Fetch recordings from xeno-canto API v3")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch a single recording by XC number
    Fetch {
        /// Xeno-canto catalogue number (e.g. 928094, XC928094, or URL)
        recording: String,

        /// Fetch metadata only (skip audio download)
        #[arg(long)]
        metadata_only: bool,

        /// Output/cache directory (default: current directory)
        #[arg(long)]
        cache_dir: Option<PathBuf>,

        /// API key (overrides stored key and XC_API_KEY env var)
        #[arg(long)]
        key: Option<String>,
    },
    /// Browse species for a group
    Browse {
        /// Group name: bats, birds, frogs, grasshoppers, "land mammals"
        group: String,

        /// Filter by country
        #[arg(long)]
        country: Option<String>,

        /// API key (overrides stored key and XC_API_KEY env var)
        #[arg(long)]
        key: Option<String>,

        /// Force refresh (ignore cache)
        #[arg(long)]
        refresh: bool,

        /// Cache directory for taxonomy data (default: current directory)
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    /// Save your XC API key (shared with the batmonic desktop app)
    SetKey {
        /// The API key to store
        key: String,
    },
    /// Show the stored API key location and status
    ShowKey,
    /// Remove the stored API key
    ClearKey,
}

fn require_api_key(explicit: &Option<String>) -> String {
    key_store::resolve_key(explicit).unwrap_or_else(|| {
        eprintln!("API key required. Options:");
        eprintln!("  xc-fetch set-key YOUR_KEY   (saves for reuse)");
        eprintln!("  --key YOUR_KEY              (one-time use)");
        eprintln!("  XC_API_KEY env var or .env   (environment)");
        std::process::exit(1);
    })
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    let client = reqwest::Client::new();

    match cli.command {
        Commands::SetKey { key } => {
            match key_store::save_key(&key) {
                Ok(path) => println!("API key saved to {}", path.display()),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::ShowKey => {
            match key_store::key_path() {
                Some(path) => {
                    println!("Key file: {}", path.display());
                    match key_store::load_key() {
                        Some(k) => {
                            let masked = if k.len() > 8 {
                                format!("{}...{}", &k[..4], &k[k.len()-4..])
                            } else {
                                "****".to_string()
                            };
                            println!("Status:   set ({})", masked);
                        }
                        None => println!("Status:   not set"),
                    }
                }
                None => println!("Could not determine config directory"),
            }
            // Also check env
            if let Ok(k) = std::env::var("XC_API_KEY") {
                if !k.is_empty() {
                    println!("Env var:  XC_API_KEY is set");
                }
            }
        }

        Commands::ClearKey => {
            match key_store::delete_key() {
                Ok(()) => println!("API key removed"),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Fetch {
            recording,
            metadata_only,
            cache_dir,
            key,
        } => {
            let api_key = require_api_key(&key);
            let xc_number = api::parse_xc_number(&recording)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });

            let cache_root = cache_dir.unwrap_or_else(|| PathBuf::from("."));

            eprintln!("Fetching XC{xc_number}...");

            let rec = api::fetch_recording(&client, &api_key, xc_number)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                });

            if metadata_only {
                let sounds_dir = cache_root.join("sounds");
                std::fs::create_dir_all(&sounds_dir).expect("Failed to create sounds dir");
                let stem = cache::recording_stem(&rec);
                let meta_filename = format!("{stem}.xc.json");
                let meta_path = sounds_dir.join(&meta_filename);
                let metadata = cache::build_metadata_json(&rec);
                let json_str = serde_json::to_string_pretty(&metadata).unwrap();
                std::fs::write(&meta_path, format!("{json_str}\n")).unwrap();
                eprintln!("Wrote {}", meta_path.display());
            } else {
                eprintln!("Downloading audio...");
                let audio_bytes = api::download_audio(&client, &rec.file_url)
                    .await
                    .unwrap_or_else(|e| {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    });

                let audio_path = cache::save_recording(&cache_root, &rec, &audio_bytes)
                    .unwrap_or_else(|e| {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    });

                eprintln!(
                    "Wrote {} ({:.1} MB)",
                    audio_path.display(),
                    audio_bytes.len() as f64 / 1_048_576.0
                );
            }

            println!("XC{}: {} ({} {})", rec.id, rec.en, rec.genus, rec.sp);
            println!("Recordist: {}", rec.rec);
            println!("License: {}", rec.lic);
            println!(
                "Attribution: {}, XC{}. Accessible at www.xeno-canto.org/{}",
                rec.rec, rec.id, rec.id
            );
        }

        Commands::Browse {
            group,
            country,
            key,
            refresh,
            cache_dir,
        } => {
            if !XC_GROUPS.contains(&group.as_str()) {
                eprintln!(
                    "Unknown group '{group}'. Available: {}",
                    XC_GROUPS.join(", ")
                );
                std::process::exit(1);
            }

            let api_key = require_api_key(&key);
            let cache_root = cache_dir.unwrap_or_else(|| PathBuf::from("."));
            let country_ref = country.as_deref();

            if !refresh {
                if let Ok(Some(cached)) = cache::load_taxonomy(&cache_root, &group, country_ref) {
                    let age = cache::taxonomy_age_string(&cache_root, &group, country_ref)
                        .unwrap_or_default();
                    eprintln!("Using cached taxonomy ({})", age);
                    print_taxonomy(&cached);
                    return;
                }
            }

            eprintln!("Fetching species list for '{group}'...");

            let taxonomy = taxonomy::build_species_list(
                &client,
                &api_key,
                &group,
                country_ref,
                |page, total| {
                    eprint!("\rPage {page}/{total}...");
                },
            )
            .await
            .unwrap_or_else(|e| {
                eprintln!("\nError: {e}");
                std::process::exit(1);
            });
            eprintln!();

            if let Err(e) = cache::save_taxonomy(&cache_root, &group, country_ref, &taxonomy) {
                eprintln!("Warning: failed to cache taxonomy: {e}");
            }

            print_taxonomy(&taxonomy);
        }
    }
}

fn print_taxonomy(taxonomy: &xc_lib::XcGroupTaxonomy) {
    println!(
        "{} species, {} recordings ({})",
        taxonomy.species.len(),
        taxonomy.total_recordings,
        taxonomy.group
    );
    println!();
    for sp in &taxonomy.species {
        println!(
            "  {:40} {:30} {:>5} recordings",
            sp.en,
            format!("{} {}", sp.genus, sp.sp),
            sp.recording_count
        );
    }
}
