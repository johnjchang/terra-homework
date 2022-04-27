#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    Binary, Deps, DepsMut, Empty, Env, MessageInfo, Response,
    StdResult, WasmMsg, BankMsg, WasmQuery, Uint128, CosmosMsg, to_binary, attr, Coin, Addr,QueryRequest
};

use cw2::set_contract_version;

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use shared::oracle::{QueryMsg as oracle_query, PriceResponse};
use shared::querier::query_balance;
use cw20::Cw20ExecuteMsg;
use crate::state::{STATE, State};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:swap";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let state: State = State{
        owner: info.sender,
        token_address: deps.api.addr_validate(msg.token_address)?,
        oracle_address: msg.oracle_address,
    };

    STATE.save(deps.storage, &state)?;

    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg{
        ExecuteMsg::Buy {} => execute_buy(deps, info, 0u64),
        ExecuteMsg::Withdraw { amount } => execute_withdraw(deps, env, info, amount),
    }
}


#[allow(clippy::too_many_arguments)]
pub fn execute_buy(
    deps: DepsMut,
    info: MessageInfo,
) -> Result<Response, ContractError> {

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
    let mut price: u64 = query_oracle(deps.as_ref(), state.oracle_address)?;

    //calc lemon quantity
    let lemons_to_sell: Uint128 = Uint128::from(1u64).multiply_ratio(luna_payment.amount, Uint128::from(price));

    //send lemon
    //cw20 mint
    let mint_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: state.token_address.into(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Mint {
            recipient: info.sender.into(),
            amount: lemons_to_sell,
        })?,
    });

    let res = Response::new()
        .add_attributes(vec![attr("action", "buy_lemons")])
        .add_message(mint_msg);

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

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: Empty) -> StdResult<Response> {
    // TODO
    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::State {} => Ok(to_binary(&query_state(deps)?)?),
    }
}

pub fn query_state(deps: Deps) -> StdResult<State> {
    let state: State = STATE.load(deps.storage)?;
    Ok(state)
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
