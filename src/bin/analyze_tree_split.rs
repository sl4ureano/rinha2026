//! Separa tree=0 vs tree=5 quando ratio_count=5 (caminho tree).
//! Busca regras 100% precisas para expandir fast_path.

use fraud_detector::ingest::extract_filled;
use fraud_detector::search::{
    complete_cache, tier_path, tree_only_count, try_fast_fraud_count, TierPath,
};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(serde::Deserialize)]
struct TestFile {
    entries: Vec<TestEntry>,
}

#[derive(serde::Deserialize)]
struct TestEntry {
    request: serde_json::Value,
}

struct Sample {
    tree0: bool,
    known: bool,
    online: bool,
    card: bool,
    safe_mcc: bool,
    risky_mcc: bool,
    amount: f32,
    installments: u32,
    tx24h: u32,
    km_home: f32,
    norm: f32,
}

fn main() {
    let data_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test/test-data.json"));

    let raw = fs::read_to_string(&data_path).unwrap();
    let file: TestFile = serde_json::from_str(&raw).unwrap();

    let mut samples = Vec::new();
    let mut tree0 = 0u64;
    let mut tree5 = 0u64;

    for entry in &file.entries {
        let body = serde_json::to_vec(&entry.request).unwrap();
        let Some(mut p) = extract_filled(&body) else {
            continue;
        };
        if try_fast_fraud_count(&p).is_some() {
            continue;
        }
        complete_cache(&mut p);
        if tier_path(&p) != TierPath::Tree || p.cache.ratio_count != 5 {
            continue;
        }

        let tc = tree_only_count(&p).unwrap();
        let is0 = tc == 0;
        if is0 {
            tree0 += 1;
        } else {
            tree5 += 1;
        }

        let c = &p.cache;
        let safe_avg = p.customer_avg_amount.max(1.0);
        let norm = ((p.amount / safe_avg) / 10.0).clamp(0.0, 1.0);
        samples.push(Sample {
            tree0: is0,
            known: c.merchant_known,
            online: p.is_online,
            card: p.card_present,
            safe_mcc: matches!(
                c.mcc_u32,
                0x3534_3131 | 0x3538_3132 | 0x3539_3132 | 0x3533_3131
            ),
            risky_mcc: matches!(c.mcc_u32, 0x3739_3935 | 0x3738_3031 | 0x3738_3032),
            amount: p.amount,
            installments: p.installments,
            tx24h: p.tx_count_24h,
            km_home: p.km_from_home,
            norm,
        });
    }

    eprintln!("ratio=5 tree path: tree0={tree0} tree5={tree5}");
    eprintln!();

    report_bool(&samples, "known", |s| s.known);
    report_bool(&samples, "!known", |s| !s.known);
    report_bool(&samples, "is_online", |s| s.online);
    report_bool(&samples, "!is_online", |s| !s.online);
    report_bool(&samples, "card_present", |s| s.card);
    report_bool(&samples, "!card_present", |s| !s.card);
    report_bool(&samples, "safe_mcc", |s| s.safe_mcc);
    report_bool(&samples, "risky_mcc", |s| s.risky_mcc);

    for thresh in [200.0, 300.0, 500.0, 800.0, 1000.0, 2000.0, 3000.0] {
        let label = format!("amount<={thresh}");
        report_bool(&samples, &label, move |s| s.amount <= thresh);
    }
    for thresh in [1, 2, 3, 4, 5] {
        let label = format!("installments<={thresh}");
        report_bool(&samples, &label, move |s| s.installments <= thresh);
    }
    for thresh in [30.0, 50.0, 80.0, 100.0, 150.0] {
        let label = format!("km<={thresh}");
        report_bool(&samples, &label, move |s| s.km_home <= thresh);
    }
    for thresh in [0.3, 0.4, 0.5, 0.6, 0.7] {
        let label = format!("norm>{thresh}");
        report_bool(&samples, &label, move |s| s.norm > thresh);
    }

    eprintln!("\n--- combos 100% tree0 (tree5=0) ---");
    search_combo_tree0(&samples);
    search_exhaustive_tree0(&samples);

    eprintln!("\n--- combos 100% tree5 (tree0=0) ---");
    search_combo_tree5(&samples);
    search_exhaustive_tree5(&samples);

    eprintln!("\n--- near-miss (tree5<=2, coverage>=20) ---");
    search_near_miss_tree0(&samples);
}

fn report_bool(samples: &[Sample], label: &str, pred: impl Fn(&Sample) -> bool) {
    let (t0, t5) = count(samples, pred);
    if t0 + t5 == 0 {
        return;
    }
    let tag = if t5 == 0 && t0 > 0 {
        " SAFE→0"
    } else if t0 == 0 && t5 > 0 {
        " SAFE→5"
    } else {
        ""
    };
    eprintln!("{label:28} tree0={t0:5} tree5={t5:5}{tag}");
}

fn count(samples: &[Sample], pred: impl Fn(&Sample) -> bool) -> (u64, u64) {
    let mut t0 = 0u64;
    let mut t5 = 0u64;
    for s in samples {
        if !pred(s) {
            continue;
        }
        if s.tree0 {
            t0 += 1;
        } else {
            t5 += 1;
        }
    }
    (t0, t5)
}

fn search_combo_tree0(samples: &[Sample]) {
    let bools: &[(&str, fn(&Sample) -> bool)] = &[
        ("known", |s| s.known),
        ("!known", |s| !s.known),
        ("online", |s| s.online),
        ("!online", |s| !s.online),
        ("card", |s| s.card),
        ("!card", |s| !s.card),
        ("safe_mcc", |s| s.safe_mcc),
        ("risky_mcc", |s| s.risky_mcc),
    ];

    for i in 0..bools.len() {
        for j in (i + 1)..bools.len() {
            let (a_name, a_fn) = bools[i];
            let (b_name, b_fn) = bools[j];
            let label = format!("{a_name}+{b_name}");
            let (t0, t5) = count(samples, |s| a_fn(s) && b_fn(s));
            if t5 == 0 && t0 >= 10 {
                eprintln!("{label:40} tree0={t0} tree5={t5} SAFE→0");
            }
        }
    }

    for amt in [500.0, 800.0, 1000.0] {
        for inst in [2, 3, 4] {
            for km in [50.0, 80.0] {
                let label = format!("known+amt<={amt}+inst<={inst}+km<={km}");
                let (t0, t5) = count(samples, |s| {
                    s.known && s.amount <= amt && s.installments <= inst && s.km_home <= km
                });
                if t5 == 0 && t0 >= 5 {
                    eprintln!("{label:40} tree0={t0} tree5={t5} SAFE→0");
                }
            }
        }
    }

    for amt in [500.0, 800.0] {
        let label = format!("known+!online+card+safe_mcc+amt<={amt}");
        let (t0, t5) = count(samples, |s| {
            s.known && !s.online && s.card && s.safe_mcc && s.amount <= amt
        });
        if t5 == 0 && t0 >= 5 {
            eprintln!("{label:40} tree0={t0} tree5={t5} SAFE→0");
        }
    }
}

fn search_exhaustive_tree0(samples: &[Sample]) {
    for known in [true, false] {
        for online in [true, false] {
            for card in [true, false] {
                for safe in [true, false] {
                    for &amt in &[500.0, 800.0, 1000.0, 1500.0, 2000.0] {
                        for inst in [2u32, 3, 4, 5] {
                            for &km in &[30.0, 50.0, 80.0, 100.0] {
                                let (t0, t5) = count(samples, |s| {
                                    s.known == known
                                        && s.online == online
                                        && s.card == card
                                        && s.safe_mcc == safe
                                        && s.amount <= amt
                                        && s.installments <= inst
                                        && s.km_home <= km
                                });
                                if t5 == 0 && t0 >= 3 {
                                    eprintln!(
                                        "k={known} on={online} card={card} safe={safe} a<={amt} i<={inst} km<={km}: tree0={t0} SAFE→0"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn search_exhaustive_tree5(samples: &[Sample]) {
    for known in [true, false] {
        for online in [true, false] {
            for card in [true, false] {
                for risky in [true, false] {
                    for &amt in &[2000.0, 3000.0, 5000.0] {
                        for inst in [3u32, 4, 5, 6] {
                            for &km in &[100.0, 150.0, 200.0] {
                                let (t0, t5) = count(samples, |s| {
                                    s.known == known
                                        && s.online == online
                                        && s.card == card
                                        && s.risky_mcc == risky
                                        && s.amount >= amt
                                        && s.installments >= inst
                                        && s.km_home >= km
                                });
                                if t0 == 0 && t5 >= 20 {
                                    eprintln!(
                                        "k={known} on={online} card={card} risky={risky} a>={amt} i>={inst} km>={km}: tree5={t5} SAFE→5"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn search_near_miss_tree0(samples: &[Sample]) {
    for known in [true, false] {
        for online in [true, false] {
            for card in [true, false] {
                for &amt in &[800.0, 1000.0, 1500.0] {
                    for inst in [3u32, 4] {
                        for &km in &[50.0, 80.0] {
                            let (t0, t5) = count(samples, |s| {
                                s.known == known
                                    && s.online == online
                                    && s.card == card
                                    && s.amount <= amt
                                    && s.installments <= inst
                                    && s.km_home <= km
                            });
                            if t0 + t5 >= 20 && t5 <= 2 {
                                eprintln!(
                                    "k={known} on={online} card={card} a<={amt} i<={inst} km<={km}: tree0={t0} tree5={t5} ({:.1}%)",
                                    100.0 * t0 as f64 / (t0 + t5) as f64
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

fn search_combo_tree5(samples: &[Sample]) {
    let bools: &[(&str, fn(&Sample) -> bool)] = &[
        ("!known", |s| !s.known),
        ("online", |s| s.online),
        ("!card", |s| !s.card),
        ("risky_mcc", |s| s.risky_mcc),
    ];

    for i in 0..bools.len() {
        for j in (i + 1)..bools.len() {
            let (a_name, a_fn) = bools[i];
            let (b_name, b_fn) = bools[j];
            let label = format!("{a_name}+{b_name}");
            let (t0, t5) = count(samples, |s| a_fn(s) && b_fn(s));
            if t0 == 0 && t5 >= 10 {
                eprintln!("{label:40} tree0={t0} tree5={t5} SAFE→5");
            }
        }
    }

    for amt in [3000.0, 5000.0] {
        for inst in [4, 5] {
            let label = format!("!known+online+!card+risky_mcc+amt>={amt}+inst>={inst}");
            let (t0, t5) = count(samples, |s| {
                !s.known
                    && s.online
                    && !s.card
                    && s.risky_mcc
                    && s.amount >= amt
                    && s.installments >= inst
            });
            if t0 == 0 && t5 >= 10 {
                eprintln!("{label:40} tree0={t0} tree5={t5} SAFE→5");
            }
        }
    }
}
