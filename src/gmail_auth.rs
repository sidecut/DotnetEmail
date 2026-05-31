use google_gmail1::{common, hyper_rustls, hyper_util, yup_oauth2, Gmail};

// C in Gmail<C> is the connector type, not the full client.
type GmailConnector =
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>;

pub type GmailHub = Gmail<GmailConnector>;

pub async fn get_gmail_service() -> Result<GmailHub, Box<dyn std::error::Error>> {
    let secret = yup_oauth2::read_application_secret("credentials.json").await?;

    // Build a connector (Clone is derived, so we can reuse it for both auth and API clients).
    let connector: GmailConnector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()?
        .https_or_http()
        .enable_http2()
        .build();

    // Build a dedicated HTTP client for the OAuth2 installed-flow authenticator.
    let auth_client = hyper_util::client::legacy::Client::builder(
        hyper_util::rt::TokioExecutor::new(),
    )
    .build(connector.clone());

    let auth = yup_oauth2::InstalledFlowAuthenticator::with_client(
        secret,
        yup_oauth2::InstalledFlowReturnMethod::HTTPRedirect,
        yup_oauth2::client::CustomHyperClientBuilder::from(auth_client),
    )
    .persist_tokens_to_disk("token.json")
    .build()
    .await?;

    println!("Credential file saved to: token.json");

    // Build the API client, explicitly typed so the body type matches common::Body.
    let client: common::Client<GmailConnector> =
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build(connector);

    Ok(Gmail::new(client, auth))
}
