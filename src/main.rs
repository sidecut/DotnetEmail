mod gmail_auth;

use chrono::{Local, NaiveDate, TimeZone};
use futures::future::join_all;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const MAX_CONCURRENT: usize = 20;

#[tokio::main]
async fn main() {
    println!("Spam Folder Email Counter");

    let args: Vec<String> = std::env::args().collect();
    let days_limit: i64 = if args.len() > 1 {
        match args[1].parse::<i64>() {
            Ok(d) => d,
            Err(_) => {
                eprintln!("Warning: invalid days argument '{}', defaulting to 30.", args[1]);
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
            if msg.contains("credentials.json") || msg.contains("os error 2") {
                eprintln!("Error: credentials.json not found. Please download it from Google Cloud Console and place it in the project directory.");
            } else {
                eprintln!("An error occurred: {}", e);
            }
        }
    }
}

async fn run(days_limit: i64, cutoff: NaiveDate) -> Result<(), Box<dyn std::error::Error>> {
    let hub = gmail_auth::get_gmail_service().await?;

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
    let total_messages = Arc::new(AtomicUsize::new(0));

    let counter_clone = Arc::clone(&total_messages);
    let timer_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(100));
        loop {
            ticker.tick().await;
            print!("\r{}", counter_clone.load(Ordering::Relaxed));
            let _ = std::io::stdout().flush();
        }
    });

    loop {
        let mut list_call = hub
            .users()
            .messages_list("me")
            .add_label_ids("SPAM")
            .include_spam_trash(true)
            .max_results(500);

        if !query.is_empty() {
            list_call = list_call.q(&query);
        }
        if let Some(ref token) = page_token {
            list_call = list_call.page_token(token);
        }

        let (_, response) = list_call.doit().await?;

        if let Some(messages) = response.messages {
            let ids: Vec<String> = messages.iter().filter_map(|m| m.id.clone()).collect();

            for chunk in ids.chunks(MAX_CONCURRENT) {
                let futures: Vec<_> = chunk
                    .iter()
                    .map(|id| hub.users().messages_get("me", id).format("minimal").doit())
                    .collect();

                let results = join_all(futures).await;

                for result in results {
                    if let Ok((_, message)) = result {
                        if let Some(epoch_ms) = message.internal_date {
                            if let Some(dt) =
                                chrono::DateTime::from_timestamp_millis(epoch_ms)
                            {
                                let local_date = dt.with_timezone(&Local).date_naive();
                                if local_date >= cutoff {
                                    *date_count_map.entry(local_date).or_insert(0) += 1;
                                    total_messages.fetch_add(1, Ordering::Relaxed);
                                }
                            }
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

    let total = total_messages.load(Ordering::Relaxed);
    println!("\rSpam emails by date:");

    if !date_count_map.is_empty() {
        let mut sorted: Vec<_> = date_count_map.iter().collect();
        sorted.sort_by_key(|(date, _)| *date);
        for (date, count) in sorted {
            println!("{} {} {}", date.format("%a"), date.format("%Y-%m-%d"), count);
        }
        println!("\nTotal: {} spam email(s)", total);
    } else {
        println!("No spam messages found.");
    }

    Ok(())
}
