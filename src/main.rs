mod gmail_auth;

use chrono::{Local, NaiveDate, TimeZone};
use futures::future::join_all;
use google_gmail1::api::Scope;
use std::collections::HashMap;
use std::io::{Error as IoError, ErrorKind, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

const MAX_CONCURRENT: usize = 20;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() {
    println!("Spam Folder Email Counter");

    let args: Vec<String> = std::env::args().collect();
    let days_limit: i64 = if args.len() > 1 {
        match args[1].parse::<i64>() {
            Ok(d) => d,
            Err(_) => {
                eprintln!(
                    "Warning: invalid days argument '{}', defaulting to 30.",
                    args[1]
                );
                30
            }
        }
    } else {
        30
    };

    let cutoff = Local::now().date_naive() - chrono::Duration::days(days_limit);
    println!(
        "Limiting to emails from the last {} day(s), i.e., since {}.",
        days_limit,
        cutoff.format("%Y-%m-%d")
    );

    match run(days_limit, cutoff).await {
        Ok(()) => {}
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("os error 21") {
                eprintln!("Error: it's likely that token.json is a directory instead of a file. Remove this directory or convert to a file.");
            } else if msg.contains("credentials.json") || msg.contains("os error 2") {
                eprintln!("Error: credentials.json not found. Please download it from Google Cloud Console and place it in the project directory.");
            } else {
                eprintln!("An error occurred: {}", e);
            }
        }
    }
}

async fn run(days_limit: i64, cutoff: NaiveDate) -> Result<(), Box<dyn std::error::Error>> {
    let hub = gmail_auth::get_gmail_service().await?;

    // Force OAuth/token acquisition once on the main path before any spawned/concurrent work.
    // This prevents multiple workers from trying to open the browser flow at the same time.
    println!("Initializing Gmail auth/token (one-time) before starting workers...");
    let _ = hub
        .users()
        .messages_list("me")
        .max_results(1)
        .add_scope(Scope::Readonly)
        .doit()
        .await?;
    println!("Gmail auth ready. Starting concurrent message processing...");

    let query = if days_limit > 0 {
        let cutoff_secs = Local
            .from_local_datetime(&cutoff.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .expect("ambiguous local time for cutoff date")
            .timestamp();
        format!("after:{}", cutoff_secs)
    } else {
        String::new()
    };

    let mut date_count_map: HashMap<NaiveDate, usize> = HashMap::new();
    let mut page_token: Option<String> = None;
    let completed_messages = Arc::new(AtomicUsize::new(0));
    let matched_messages = Arc::new(AtomicUsize::new(0));
    let failed_messages = Arc::new(AtomicUsize::new(0));

    let completed_clone = Arc::clone(&completed_messages);
    let matched_clone = Arc::clone(&matched_messages);
    let failed_clone = Arc::clone(&failed_messages);
    let timer_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(100));
        loop {
            ticker.tick().await;
            print!(
                "\rCompleted: {}  Matched: {}  Failed: {}",
                completed_clone.load(Ordering::Relaxed),
                matched_clone.load(Ordering::Relaxed),
                failed_clone.load(Ordering::Relaxed)
            );
            let _ = std::io::stdout().flush();
        }
    });

    loop {
        let mut list_call = hub
            .users()
            .messages_list("me")
            .add_label_ids("SPAM")
            .include_spam_trash(true)
            .max_results(500)
            .add_scope(Scope::Readonly);

        if !query.is_empty() {
            list_call = list_call.q(&query);
        }
        if let Some(ref token) = page_token {
            list_call = list_call.page_token(token);
        }

        let (_, response) = timeout(REQUEST_TIMEOUT, list_call.doit())
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::TimedOut,
                    "Timed out while listing Gmail spam messages",
                )
            })??;

        if let Some(messages) = response.messages {
            let ids: Vec<String> = messages.iter().filter_map(|m| m.id.clone()).collect();

            for chunk in ids.chunks(MAX_CONCURRENT) {
                let futures: Vec<_> = chunk
                    .iter()
                    .map(|id| async {
                        let id = id.clone();
                        let result = timeout(
                            REQUEST_TIMEOUT,
                            hub.users()
                                .messages_get("me", &id)
                                .format("minimal")
                                .add_scope(Scope::Readonly)
                                .doit(),
                        )
                        .await;

                        (id, result)
                    })
                    .collect();

                let results = join_all(futures).await;

                for (id, result) in results {
                    completed_messages.fetch_add(1, Ordering::Relaxed);

                    match result {
                        Ok(Ok((_, message))) => {
                            if let Some(epoch_ms) = message.internal_date {
                                if let Some(dt) = chrono::DateTime::from_timestamp_millis(epoch_ms)
                                {
                                    let local_date = dt.with_timezone(&Local).date_naive();
                                    if local_date >= cutoff {
                                        *date_count_map.entry(local_date).or_insert(0) += 1;
                                        matched_messages.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                        Ok(Err(err)) => {
                            failed_messages.fetch_add(1, Ordering::Relaxed);
                            eprintln!("\nWarning: failed to fetch message {id}: {err}");
                        }
                        Err(_) => {
                            failed_messages.fetch_add(1, Ordering::Relaxed);
                            eprintln!(
                                "\nWarning: timed out fetching message {id} after {} seconds; skipping.",
                                REQUEST_TIMEOUT.as_secs()
                            );
                        }
                    }
                }
            }
        }

        page_token = response.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    timer_handle.abort();

    let total = matched_messages.load(Ordering::Relaxed);
    let failed = failed_messages.load(Ordering::Relaxed);
    println!("\rSpam emails by date:");

    if !date_count_map.is_empty() {
        let mut sorted: Vec<_> = date_count_map.iter().collect();
        sorted.sort_by_key(|(date, _)| *date);
        for (date, count) in sorted {
            println!(
                "{} {} {}",
                date.format("%a"),
                date.format("%Y-%m-%d"),
                count
            );
        }
        println!("\nTotal: {} spam email(s)", total);
    } else {
        println!("No spam messages found.");
    }

    if failed > 0 {
        println!(
            "Skipped {} message request(s) due to errors or timeouts.",
            failed
        );
    }

    Ok(())
}
