use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, program::invoke_signed},
};
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};
use jupiter_aggregator::program::Jupiter;
use std::str::FromStr;

declare_program!(jupiter_aggregator);
declare_id!("8KQG1MYXru73rqobftpFjD3hBD8Ab3jaag8wbjZG63sx");

const VAULT_SEED: &[u8] = b"vault";

pub fn jupiter_program_id() -> Pubkey {
    Pubkey::from_str("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4").unwrap()
}

#[program]
pub mod cpi_swap_program {
    use super::*;

    pub fn swap(ctx: Context<Swap>, data: Vec<u8>) -> Result<()> {
        msg!("1");
        // TODO: Check the first 8 bytes. Only Jupiter Route CPI allowed.
        require_keys_eq!(*ctx.accounts.jupiter_program.key, jupiter_program_id());

        let accounts: Vec<AccountMeta> = ctx
            .remaining_accounts
            .iter()
            .map(|acc| {
                let is_signer = acc.key == &ctx.accounts.signer.key();
                AccountMeta {
                    pubkey: *acc.key,
                    is_signer,
                    is_writable: acc.is_writable,
                }
            })
            .collect();

        msg!("2");
        let accounts_infos: Vec<AccountInfo> = ctx
            .remaining_accounts
            .iter()
            .map(|acc| AccountInfo { ..acc.clone() })
            .collect();

        msg!("3");
        let signer_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, &[ctx.bumps.vault]]];
        // Execute swap
        msg!("4");

        invoke_signed(
            &Instruction {
                program_id: ctx.accounts.jupiter_program.key(),
                accounts,
                data,
            },
            &accounts_infos,
            signer_seeds,
        )?;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Swap<'info> {
    pub signer: Signer<'info>,
    pub input_mint: InterfaceAccount<'info, Mint>,
    pub input_mint_program: Interface<'info, TokenInterface>,
    pub output_mint: InterfaceAccount<'info, Mint>,
    pub output_mint_program: Interface<'info, TokenInterface>,

    #[account(
      mut,
      seeds=[VAULT_SEED],
      bump
    )]
    pub vault: SystemAccount<'info>,

    #[account(
      mut,
      associated_token::mint=input_mint,
      associated_token::authority=vault,
      associated_token::token_program=input_mint_program,
    )]
    pub vault_input_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(
      mut,
      associated_token::mint=output_mint,
      associated_token::authority=vault,
      associated_token::token_program=output_mint_program,
    )]
    pub vault_output_token_account: InterfaceAccount<'info, TokenAccount>,

    /// CHECK: testing
    pub jupiter_program: Program<'info, Jupiter>,
}
