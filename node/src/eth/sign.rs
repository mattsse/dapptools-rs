use crate::eth::error::BlockchainError;
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{Address, Wallet},
    types::{transaction::eip2718::TypedTransaction as EthersTypedTransactionRequest, H256},
};
use foundry_node_core::eth::transaction::{
    EIP1559Transaction, EIP1559TransactionRequest, EIP2930Transaction, EIP2930TransactionRequest,
    LegacyTransaction, LegacyTransactionRequest, TypedTransaction, TypedTransactionRequest,
};
use std::collections::HashMap;

/// A transaction signer
pub trait Signer: Send + Sync {
    /// returns the available accounts for this signer
    fn accounts(&self) -> Vec<Address>;
    /// signs a transaction request using the given account in request
    fn sign(
        &self,
        request: TypedTransactionRequest,
        address: &Address,
    ) -> Result<TypedTransaction, BlockchainError>;
}

pub struct DevSigner {
    accounts: HashMap<Address, Wallet<SigningKey>>,
}

impl DevSigner {
    pub fn new(accounts: HashMap<Address, Wallet<SigningKey>>) -> Self {
        Self { accounts }
    }
}

impl Signer for DevSigner {
    fn accounts(&self) -> Vec<Address> {
        self.accounts.keys().copied().collect()
    }

    fn sign(
        &self,
        request: TypedTransactionRequest,
        address: &Address,
    ) -> Result<TypedTransaction, BlockchainError> {
        let signer = self.accounts.get(address).ok_or(BlockchainError::NoSignerAvailable)?;

        let ethers_tx: EthersTypedTransactionRequest = request.clone().into();

        let signature = signer.sign_transaction_sync(&ethers_tx);

        let tx = match request {
            TypedTransactionRequest::Legacy(tx) => {
                let LegacyTransactionRequest {
                    nonce,
                    gas_price,
                    gas_limit,
                    kind,
                    value,
                    input,
                    ..
                } = tx;
                TypedTransaction::Legacy(LegacyTransaction {
                    nonce,
                    gas_price,
                    gas_limit,
                    kind,
                    value,
                    input,
                    signature,
                })
            }
            TypedTransactionRequest::EIP2930(tx) => {
                let EIP2930TransactionRequest {
                    chain_id,
                    nonce,
                    gas_price,
                    gas_limit,
                    kind,
                    value,
                    input,
                    access_list,
                } = tx;

                let recid: u8 = signature.recovery_id()?.into();

                TypedTransaction::EIP2930(EIP2930Transaction {
                    chain_id,
                    nonce,
                    gas_price,
                    gas_limit,
                    kind,
                    value,
                    input,
                    access_list: access_list.into(),
                    odd_y_parity: recid != 0,
                    r: {
                        let mut rarr = [0_u8; 32];
                        signature.r.to_big_endian(&mut rarr);
                        H256::from(rarr)
                    },
                    s: {
                        let mut sarr = [0_u8; 32];
                        signature.s.to_big_endian(&mut sarr);
                        H256::from(sarr)
                    },
                })
            }
            TypedTransactionRequest::EIP1559(tx) => {
                let EIP1559TransactionRequest {
                    chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    kind,
                    value,
                    input,
                    access_list,
                } = tx;

                let recid: u8 = signature.recovery_id()?.into();

                TypedTransaction::EIP1559(EIP1559Transaction {
                    chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    kind,
                    value,
                    input,
                    access_list: access_list.into(),
                    odd_y_parity: recid != 0,
                    r: {
                        let mut rarr = [0_u8; 32];
                        signature.r.to_big_endian(&mut rarr);
                        H256::from(rarr)
                    },
                    s: {
                        let mut sarr = [0_u8; 32];
                        signature.s.to_big_endian(&mut sarr);
                        H256::from(sarr)
                    },
                })
            }
        };

        Ok(tx)
    }
}
