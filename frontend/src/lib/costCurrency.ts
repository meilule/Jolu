import { useEffect, useState } from "react"
import { api, isTauri, listenEvent } from "./api"
import { EVENTS } from "./events"

export type CostCurrency = "USD" | "CNY"

export interface FxState {
  currency: CostCurrency
  /** 1 USD 兑目标币种；USD 时恒为 1 */
  rate: number
  /** 汇率来源说明（日期、「自定义固定汇率」或「内置参考价」） */
  asOf: string
  fallback: boolean
  fixed: boolean
  liveRate?: number | null
  liveAsOf?: string | null
  loading: boolean
}

const EMPTY: FxState = { currency: "USD", rate: 1, asOf: "基准币种", fallback: false, fixed: false, loading: false }

/** 按设置币种拉汇率；USD 恒为基准，支持自动汇率与自定义固定汇率联动刷新。 */
export function useCostFx(currency: CostCurrency, refreshKey = ""): FxState {
  const [nonce, setNonce] = useState(0)
  const [state, setState] = useState<FxState>(() =>
    currency === "USD" ? EMPTY : { currency, rate: 7.2, asOf: "读取中…", fallback: false, fixed: false, loading: true },
  )

  useEffect(() => {
    if (!isTauri) return
    let disposed = false
    let off: (() => void) | undefined
    void listenEvent(EVENTS.PRICING_CHANGED, () => { if (!disposed) setNonce((n) => n + 1) })
      .then((un) => { if (disposed) un(); else off = un })
      .catch(() => {})
    return () => { disposed = true; off?.() }
  }, [])

  useEffect(() => {
    if (currency === "USD") {
      setState(EMPTY)
      return
    }
    if (!isTauri) {
      setState({ currency: "CNY", rate: 7.25, asOf: "自定义固定汇率", fallback: false, fixed: true, liveRate: 7.21, liveAsOf: "今日参考", loading: false })
      return
    }
    let stopped = false
    setState((prev) => ({ ...prev, currency, loading: true }))
    api
      .fxRate(currency)
      .then((dto) => {
        if (!stopped) {
          setState({
            currency,
            rate: dto.rate,
            asOf: dto.as_of,
            fallback: dto.fallback,
            fixed: !!dto.fixed,
            liveRate: dto.live_rate ?? null,
            liveAsOf: dto.live_as_of ?? null,
            loading: false,
          })
        }
      })
      .catch(() => {
        if (!stopped) {
          setState({ currency, rate: 7.2, asOf: "离线参考价", fallback: true, fixed: false, loading: false })
        }
      })
    return () => {
      stopped = true
    }
  }, [currency, refreshKey, nonce])

  return state
}

export { convertFromUsd } from "./usageStats"
