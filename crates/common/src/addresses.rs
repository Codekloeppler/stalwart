/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-SEL
 */

use directory::{Directory, backend::RcptType};
use std::borrow::Cow;
use utils::config::{Config, utils::AsKey};

use crate::{
    Server,
    config::smtp::session::AddressMapping,
    expr::{
        V_RECIPIENT, Variable, functions::ResolveVariable, if_block::IfBlock, tokenizer::TokenMap,
    },
};

impl Server {
    pub async fn email_to_id(
        &self,
        directory: &Directory,
        email: &str,
        session_id: u64,
    ) -> trc::Result<Option<u32>> {
        let mut address = self
            .core
            .smtp
            .session
            .rcpt
            .subaddressing
            .to_subaddress(self, email, session_id)
            .await;

        for _ in 0..2 {
            let result = directory.email_to_id(address.as_ref()).await?;

            if result.is_some() {
                return Ok(result);
            } else if let Some(catch_all) = self
                .core
                .smtp
                .session
                .rcpt
                .catch_all
                .to_catch_all(self, email, session_id)
                .await
            {
                address = catch_all;
            } else {
                break;
            }
        }

        Ok(None)
    }

    pub async fn rcpt(
        &self,
        directory: &Directory,
        email: &str,
        session_id: u64,
    ) -> trc::Result<RcptType> {
        // Expand subaddress
        let mut address = self
            .core
            .smtp
            .session
            .rcpt
            .subaddressing
            .to_subaddress(self, email, session_id)
            .await;

        for _ in 0..2 {
            let rcpt_type = directory.rcpt(address.as_ref()).await?;
            if rcpt_type != RcptType::Invalid {
                return Ok(rcpt_type);
            } else if let Some(catch_all) = self
                .core
                .smtp
                .session
                .rcpt
                .catch_all
                .to_catch_all(self, email, session_id)
                .await
            {
                address = catch_all;
            } else {
                break;
            }
        }

        Ok(RcptType::Invalid)
    }

    pub async fn vrfy(
        &self,
        directory: &Directory,
        address: &str,
        session_id: u64,
    ) -> trc::Result<Vec<String>> {
        directory
            .vrfy(
                self.core
                    .smtp
                    .session
                    .rcpt
                    .subaddressing
                    .to_subaddress(self, address, session_id)
                    .await
                    .as_ref(),
            )
            .await
    }

    pub async fn expn(
        &self,
        directory: &Directory,
        address: &str,
        session_id: u64,
    ) -> trc::Result<Vec<String>> {
        directory
            .expn(
                self.core
                    .smtp
                    .session
                    .rcpt
                    .subaddressing
                    .to_subaddress(self, address, session_id)
                    .await
                    .as_ref(),
            )
            .await
    }
}

impl AddressMapping {
    pub fn parse(config: &mut Config, key: impl AsKey) -> Self {
        let key = key.as_key();
        if let Some(value) = config.value(key.as_str()) {
            match value {
                "true" => AddressMapping::Enable,
                "false" => AddressMapping::Disable,
                _ => {
                    config.new_parse_error(
                        key,
                        format!("Invalid value for address mapping {value:?}",),
                    );
                    AddressMapping::Disable
                }
            }
        } else if let Some(if_block) = IfBlock::try_parse(
            config,
            key,
            &TokenMap::default().with_variables_map([
                ("address", V_RECIPIENT),
                ("email", V_RECIPIENT),
                ("rcpt", V_RECIPIENT),
            ]),
        ) {
            AddressMapping::Custom(if_block)
        } else {
            AddressMapping::Enable
        }
    }
}

struct Address<'x>(&'x str);

impl ResolveVariable for Address<'_> {
    fn resolve_variable(&self, _: u32) -> crate::expr::Variable {
        Variable::from(self.0)
    }

    fn resolve_global(&self, _: &str) -> Variable<'_> {
        Variable::Integer(0)
    }
}

impl AddressMapping {
    pub async fn to_subaddress<'x, 'y: 'x>(
        &'x self,
        core: &Server,
        address: &'y str,
        session_id: u64,
    ) -> Cow<'x, str> {
        match self {
            AddressMapping::Enable => {
                if let Some((local_part, domain_part)) = address.rsplit_once('@') {
                    if let Some((local_part, _)) = local_part.split_once('+') {
                        return format!("{}@{}", local_part, domain_part).into();
                    }
                }
            }
            AddressMapping::Custom(if_block) => {
                if let Some(result) = core
                    .eval_if::<String, _>(if_block, &Address(address), session_id)
                    .await
                {
                    return result.into();
                }
            }
            AddressMapping::Disable => (),
        }

        address.into()
    }

    pub async fn to_catch_all<'x, 'y: 'x>(
        &'x self,
        core: &Server,
        address: &'y str,
        session_id: u64,
    ) -> Option<Cow<'x, str>> {
        match self {
            AddressMapping::Enable => address
                .rsplit_once('@')
                .map(|(_, domain_part)| format!("@{}", domain_part))
                .map(Cow::Owned),
            AddressMapping::Custom(if_block) => core
                .eval_if::<String, _>(if_block, &Address(address), session_id)
                .await
                .map(Cow::Owned),
            AddressMapping::Disable => None,
        }
    }
}
