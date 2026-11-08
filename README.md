# ProofStream on NEAR

Funded milestones on NEAR, paid only when the hard facts of a merged pull request pass and neither of two independent AI reviewers objects. Work in progress; the full README comes with the build.

## Next steps

Pay contributors on any chain. The gist names an address on Base, Solana or another chain, and the payout routes through NEAR Intents. Before this ships, the Intents deposit address has to be fetched and bound inside the enclave, so no relayer can substitute its own. Mainnet only, since NEAR Intents has no testnet.

Fund milestones from any chain. USDC arrives through NEAR Intents’ customRecipientMsg, which calls the contract’s ft_transfer_call. This needs an exact-output quote and a funding message that names the funder, because the sender is Intents’ withdrawal account. NEAR Intents marks this path as experimental.

Optional yield on escrowed budgets. An opt-in setting would supply idle milestone budgets to RHEA Lending through @rhea-finance/cross-chain-sdk and withdraw them before payout. Disclaimer: like any DeFi integration, this would be off by default, and the funder chooses whether to use it.
