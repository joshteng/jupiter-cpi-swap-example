mod helpers;
mod retryable_rpc;

use base64::engine::Engine;
use borsh::{BorshDeserialize, BorshSerialize};
use helpers::{get_address_lookup_table_accounts, get_discriminator};
use jup_swap::{
    quote::QuoteRequest, swap::SwapRequest, transaction_config::TransactionConfig,
    JupiterSwapApiClient,
};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction},
    message::{v0::Message, VersionedMessage},
    pubkey,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::VersionedTransaction,
};
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use spl_token::ID as TOKEN_PROGRAM_ID;
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio;
use tokio::sync::RwLock;

const INPUT_MINT: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const INPUT_AMOUNT: u64 = 1_000_000;
const OUTPUT_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
// const SLIPPAGE_BPS: u16 = 100;

const CPI_SWAP_PROGRAM_ID: Pubkey = pubkey!("8KQG1MYXru73rqobftpFjD3hBD8Ab3jaag8wbjZG63sx");
const JUPITER_PROGRAM_ID: Pubkey = pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
// const DEFAULT_RPC_URL: &str = "http://127.0.0.1:8899";
const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";

struct LatestBlockhash {
    blockhash: RwLock<solana_sdk::hash::Hash>,
    slot: AtomicU64,
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct SwapIxData {
    pub data: Vec<u8>,
}

#[tokio::main]
async fn main() {
    let rpc_url = env::var("RPC_URL").unwrap_or(DEFAULT_RPC_URL.to_string());

    let keypair_str = env::var("KEYPAIR").expect("KEYPAIR environment variable not set");
    let keypair_bytes: Vec<u8> = keypair_str
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().parse().expect("Failed to parse u8 value"))
        .collect();
    let keypair = Keypair::from_bytes(&keypair_bytes).unwrap();

    let rpc_client = Arc::new(RpcClient::new_with_commitment(
        rpc_url.to_string(),
        CommitmentConfig::confirmed(),
    ));

    let rpc_client_clone = rpc_client.clone();
    let latest_blockhash = Arc::new(LatestBlockhash {
        blockhash: RwLock::new(solana_sdk::hash::Hash::default()),
        slot: AtomicU64::new(0),
    });

    let latest_blockhash_clone = latest_blockhash.clone();
    tokio::spawn(async move {
        loop {
            if let Ok((blockhash, slot)) =
                rpc_client_clone.get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
            {
                let mut blockhash_write = latest_blockhash_clone.blockhash.write().await;
                *blockhash_write = blockhash;
                latest_blockhash_clone.slot.store(slot, Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    let api_base_url = env::var("API_BASE_URL").unwrap_or("https://quote-api.jup.ag/v6".into());

    let jupiter_swap_api_client = JupiterSwapApiClient::new(api_base_url);

    let quote_request = QuoteRequest {
        amount: INPUT_AMOUNT,
        input_mint: INPUT_MINT,
        output_mint: OUTPUT_MINT,
        // slippage_bps: SLIPPAGE_BPS,
        ..QuoteRequest::default()
    };

    // GET /quote
    println!("Getting quote...");
    let quote_response = match jupiter_swap_api_client.quote(&quote_request).await {
        Ok(quote_response) => quote_response,
        Err(e) => {
            println!("quote failed: {e:#?}");
            return;
        }
    };
    // println!("{quote_response:#?}");

    // POST /swap-instructions
    println!("Getting swap instructions...");
    let (vault, _) = Pubkey::find_program_address(&[b"vault"], &CPI_SWAP_PROGRAM_ID);

    let response = jupiter_swap_api_client
        .swap_instructions(&SwapRequest {
            user_public_key: vault,
            quote_response,
            config: TransactionConfig {
                skip_user_accounts_rpc_calls: true,
                wrap_and_unwrap_sol: false,
                use_shared_accounts: Some(true),
                ..TransactionConfig::default()
            },
        })
        .await
        .unwrap();

    println!("Getting address lookup table accounts...");
    let address_lookup_table_accounts =
        get_address_lookup_table_accounts(&rpc_client, response.address_lookup_table_addresses)
            .await
            .unwrap();

    println!("Vault: {}", vault);
    let input_token_account = get_associated_token_address(&vault, &INPUT_MINT);
    let output_token_account = get_associated_token_address(&vault, &OUTPUT_MINT);

    let create_output_ata_ix = create_associated_token_account_idempotent(
        &keypair.pubkey(),
        &vault,
        &OUTPUT_MINT,
        &TOKEN_PROGRAM_ID,
    );

    let instruction_data = SwapIxData {
        data: response.swap_instruction.data,
    };

    let mut serialized_data = Vec::from(get_discriminator("global:swap"));
    instruction_data.serialize(&mut serialized_data).unwrap();

    let mut accounts = vec![
        AccountMeta::new(keypair.pubkey(), true),
        AccountMeta::new_readonly(INPUT_MINT, false), // input mint
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false), // input mint program (for now, just hardcoded to SPL and not SPL 2022)
        AccountMeta::new_readonly(OUTPUT_MINT, false),      // output mint
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false), // output mint program (for now, just hardcoded to SPL and not SPL 2022)
        AccountMeta::new(vault, false),                     // vault
        AccountMeta::new(input_token_account, false),       // vault input token account
        AccountMeta::new(output_token_account, false),      // vault output token account
        AccountMeta::new_readonly(JUPITER_PROGRAM_ID, false), // jupiter program
    ];
    let remaining_accounts = response.swap_instruction.accounts;
    accounts.extend(remaining_accounts.into_iter().map(|mut account| {
        account.is_signer = false;
        account
    }));

    let swap_ix = Instruction {
        program_id: CPI_SWAP_PROGRAM_ID,
        accounts,
        data: serialized_data,
    };

    let cu_ix = ComputeBudgetInstruction::set_compute_unit_limit(500_000);
    let cup_ix = ComputeBudgetInstruction::set_compute_unit_price(100_000);
    // let heap_ix = ComputeBudgetInstruction::request_heap_frame(32768);

    loop {
        let slot = latest_blockhash.slot.load(Ordering::Relaxed);
        if slot != 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let latest_blockhash = latest_blockhash.blockhash.read().await;
    println!("Latest blockhash: {}", latest_blockhash);
    let message = Message::try_compile(
        &keypair.pubkey(),
        &[cu_ix, cup_ix, create_output_ata_ix, swap_ix],
        &address_lookup_table_accounts,
        *latest_blockhash,
    )
    .unwrap();

    println!(
        "Base64 EncodedTransaction message: {}",
        base64::engine::general_purpose::STANDARD
            .encode(VersionedMessage::V0(message.clone()).serialize())
    );
    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[keypair]).unwrap();
    let retryable_client = retryable_rpc::RetryableRpcClient::new(&rpc_url);

    let tx_hash = tx.signatures[0];

    if let Ok(tx_hash) = retryable_client.send_and_confirm_transaction(&tx).await {
        println!(
            "Transaction confirmed: https://explorer.solana.com/tx/{}",
            tx_hash
        );
    } else {
        println!(
            "Transaction failed: https://explorer.solana.com/tx/{}",
            tx_hash
        );
        return;
    };
}
