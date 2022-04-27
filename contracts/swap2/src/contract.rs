#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    Binary, Deps, DepsMut,
    Empty, Env, MessageInfo, Response, StdError, StdResult, SubMsg, Uint128, attr, Coin,
    CosmosMsg, to_binary, WasmMsg, StakingMsg, WasmQuery, QueryRequest, Addr, DistributionMsg, BankMsg
};
use cw0::must_pay;
use cw2::set_contract_version;
use cw20::Cw20ExecuteMsg;
use shared::oracle::{QueryMsg as oracle_query, PriceResponse};

use terra_cosmwasm::{ExchangeRatesResponse, TerraMsgWrapper, TerraQuerier, create_swap_msg};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{STATE, State};
use shared::querier::*;

// use oracle::contract::query_price;
// use oracle::msg::PriceResponse;

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:swap2";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// BlockNgine - 0% comission on testnet
const _VALIDATOR: &str = "terravaloper1ze5dxzs4zcm60tg48m9unp8eh7maerma38dl84";

// StakeBin - 1% comission on testnet
// https://finder.terra.money/testnet/validator/terravaloper19ne0aqltndwxl0n32zyuglp2z8mm3nu0gxpfaw
// const VALIDATOR: &str = "terravaloper19ne0aqltndwxl0n32zyuglp2z8mm3nu0gxpfaw";

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let state: State = State{
        owner: info.sender,
        token_address: msg.token_address,
        oracle_address: msg.oracle_address,
    };

    STATE.save(deps.storage, &state)?;

    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(_deps: Deps, _env: Env, _msg: QueryMsg) -> StdResult<Binary> {
    // TODO
    Err(StdError::generic_err("not implemented"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: Empty) -> Result<Response, ContractError> {
    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response<TerraMsgWrapper>, ContractError> {
    match msg {
        // Buy
        ExecuteMsg::Buy {} => try_buy(deps, env, info),

        // Withdraw
        ExecuteMsg::Withdraw { amount } => try_withdraw_step1_collect_rewards(deps, env, info, amount), // Step 1: claim rewards from validators
        ExecuteMsg::WithdrawStep2ConvertRewardsToLuna {  } => try_withdraw_step2_convert_all_native_coins_to_luna(deps, env, info),
        ExecuteMsg::WithdrawStep3SendLuna { amount } => try_withdraw_step3_send_luna(deps, env, info, amount),

        // StartUndelegation
        ExecuteMsg::StartUndelegation { amount } => try_start_undelegation(deps, env, info, amount),
    }
}

pub fn try_buy(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response<TerraMsgWrapper>, ContractError> {

    //fetch luna sent
    let luna_payment: &Coin = 
        info
            .funds
            .iter()
            .find(|x| x.denom == String::from("uluna") && x.amount > Uint128::zero())
            .ok_or_else(||{
                ContractError::InvalidQuantity{}
        })?;

    //fetch state
    let state: State = STATE.load(deps.storage)?;

    //query oracle price
    let price: u64 = query_oracle(deps.as_ref(), state.oracle_address.clone())?;
    
    //calc lemon quantity
    let lemons_to_sell: Uint128 = Uint128::from(1u64).multiply_ratio(luna_payment.amount, Uint128::from(price));

    //send lemon
    //cw20 mint
    let mint_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: state.token_address.into(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Transfer {
            recipient: info.sender.into(),
            amount: lemons_to_sell,
        })?,
    });

    //delegate luna
    let delegate_msg = CosmosMsg::Staking(StakingMsg::Delegate {
        validator: String::from(_VALIDATOR),
        amount: Coin::new(luna_payment.amount.into(), "uluna"),
    });


    let res = Response::new()
        .add_attributes(vec![attr("action", "buy_lemons")])
        .add_messages(vec![mint_msg, delegate_msg]);

    Ok(res)
}

pub fn try_withdraw_step1_collect_rewards(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: u64,
) -> Result<Response<TerraMsgWrapper>, ContractError> {

    //priv check
    let state: State = STATE.load(deps.storage)?;

    if state.owner != info.sender {
        return Err(ContractError::Unauthorized{});
    }

    // Step 1: Collect all rewards we have accrued.
    let mut submessages: Vec<SubMsg<TerraMsgWrapper>> = vec![];

    //fabricate reward collection sub-messages
    let mut reward_submessages = collect_all_rewards(deps, &env)?;
    submessages.append(&mut reward_submessages);

    //fabricate swap submessage
    submessages.push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute{
        contract_addr: env.contract.address.clone().into(),
        msg: to_binary(&ExecuteMsg::WithdrawStep2ConvertRewardsToLuna{})?,
        funds: vec![],
    })));

    //fabricate send luna message
    let send_msg = CosmosMsg::Wasm(WasmMsg::Execute{
        contract_addr: env.contract.address.into(),
        funds: vec![],
        msg: to_binary(&ExecuteMsg::WithdrawStep3SendLuna{amount})?,
    });


    // TODO
    Ok(Response::<TerraMsgWrapper>::new()
        .add_submessages(reward_submessages)
        .add_message(send_msg))
}

pub fn collect_all_rewards(
    deps: DepsMut,
    env: &Env,
) -> Result<Vec<SubMsg<TerraMsgWrapper>>, ContractError> {

    //stolen from basset hub
    let mut messages: Vec<SubMsg<TerraMsgWrapper>> = vec![];
    let delegations = deps.querier.query_all_delegations(env.contract.address.clone());
    
    if let Ok(delegations) = delegations{
        for delegation in delegations{
            let msg: CosmosMsg = CosmosMsg::Distribution(DistributionMsg::WithdrawDelegatorReward{
                validator: delegation.validator,
            });
        }
    }

    Ok(messages)
}


pub fn try_withdraw_step2_convert_all_native_coins_to_luna(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response<TerraMsgWrapper>, ContractError> {

    //priv check
    if info.sender != env.contract.address {
        return Err(ContractError::Unauthorized{});
    }

    let balance = deps.querier.query_all_balances(env.contract.address.clone())?;
    let mut messages: Vec<CosmosMsg<TerraMsgWrapper>> = Vec::new();

    let denoms: Vec<String> = balance.iter().map(|item| item.denom.clone()).collect();

    let exchange_rates = query_exchange_rates(&deps, String::from("uluna"), denoms)?;

    let known_denoms: Vec<String> = exchange_rates
        .exchange_rates
        .iter()
        .map(|item| item.quote_denom.clone())
        .collect();

    for coin in balance {
        if coin.denom == String::from("uluna") || !known_denoms.contains(&coin.denom) {
            continue;
        }

        messages.push(create_swap_msg(coin, String::from("uluna")));
    }

    let res = Response::new()
    .add_messages(messages)
    .add_attributes(vec![attr("action", "swap")]);

    Ok(res)
}


pub fn try_withdraw_step3_send_luna(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: u64,
) -> Result<Response<TerraMsgWrapper>, ContractError> {

    //priv check
    let state: State = STATE.load(deps.storage)?;

    if env.contract.address != info.sender {
        return Err(ContractError::Unauthorized{});
    }

    //check balance
    let balance: Uint128 = query_balance(&deps.querier, &env.contract.address, String::from("uluna"))?;

    if balance < amount.into(){
        return Err(ContractError::InvalidQuantity{});
    }

    //pay out uluna
    let bank_msg = CosmosMsg::Bank(BankMsg::Send{
        to_address: info.sender.to_string(),
        amount: vec![
            Coin{
                denom: String::from("uluna"),
                amount: amount.into(),
            }],
    });


    let res = Response::new()
        .add_attributes(vec![attr("action", "withdraw_luna")])
        .add_message(bank_msg);

    Ok(res)
}

#[allow(clippy::too_many_arguments)]
pub fn execute_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: i32,
) -> Result<Response, ContractError> {

    //priv check
    let state: State = STATE.load(deps.storage)?;

    if state.owner != info.sender {
        return Err(ContractError::Unauthorized{});
    }

    //valid amount check
    if amount <= 0i32{
        return Err(ContractError::InvalidQuantity{});
    }

    //sketchy convert
    let amount: u64 = amount as u64;

    //check balance
    let balance: Uint128 = query_balance(&deps.querier, &env.contract.address, String::from("uluna"))?;

    if balance < amount.into(){
        return Err(ContractError::InvalidQuantity{});
    }

    //pay out uluna
    let bank_msg = CosmosMsg::Bank(BankMsg::Send{
        to_address: info.sender.to_string(),
        amount: vec![
            Coin{
                denom: String::from("uluna"),
                amount: amount.into(),
            }],
    });


    let res = Response::new()
        .add_attributes(vec![attr("action", "withdraw_luna")])
        .add_message(bank_msg);

    Ok(res)
}

pub fn try_start_undelegation(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _amount: Uint128,
) -> Result<Response<TerraMsgWrapper>, ContractError> {
    // TODO
    Err(ContractError::NotImplemented {})
}

pub fn query_exchange_rates(
    deps: &DepsMut,
    base_denom: String,
    quote_denoms: Vec<String>,
) -> StdResult<ExchangeRatesResponse> {
    let querier = TerraQuerier::new(&deps.querier);
    let res: ExchangeRatesResponse = querier.query_exchange_rates(base_denom, quote_denoms)?;
    Ok(res)
}


pub fn query_oracle(deps: Deps, oracle_address: Addr) -> StdResult<u64> {
    // load price form the oracle
    let price_response: PriceResponse =
        deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: oracle_address.to_string(),
            msg: to_binary(&oracle_query::QueryPrice {
            })?,
        }))?;

    Ok(price_response.price)
}


#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coins, Addr};

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg {
            token_address: Addr::unchecked("terra1hpajld8zs93md8zrs6sfy42zl0khqpmr07muw0"),
        };
        let info = mock_info("creator", &coins(10000000000, "uluna"));

        // we can just call .unwrap() to assert this was a success
        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::QueryTokenAddress {});
        assert_eq!(res, Err(StdError::generic_err("not implemented")));

        // let value: QueryTokenAddressResponse = from_binary(&res).unwrap();
        // assert_eq!(
        //     "terra1hpajld8zs93md8zrs6sfy42zl0khqpmr07muw0",
        //     value.token_address
        // );
    }
}
