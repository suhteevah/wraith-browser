'use client';

import { useState } from 'react';

const CURRENT_SOLUTIONS = [
  { name: 'Browserbase', costPer1k: 10.00 },
  { name: 'Browserless', costPer1k: 8.00 },
  { name: 'Playwright (self-hosted)', costPer1k: 3.50 },
  { name: 'Puppeteer (self-hosted)', costPer1k: 3.50 },
  { name: 'Custom scraping infra', costPer1k: 5.00 },
] as const;

const WRAITH_TIERS = [
  { name: 'Self-hosted', monthlyBase: 0, includedPages: Infinity, overagePer1k: 0, maxPages: Infinity },
  { name: 'Growth', monthlyBase: 199, includedPages: 100_000, overagePer1k: 1.50, maxPages: 500_000 },
  { name: 'Scale', monthlyBase: 799, includedPages: 1_000_000, overagePer1k: 0.60, maxPages: 10_000_000 },
  { name: 'Enterprise', monthlyBase: 2499, includedPages: 10_000_000, overagePer1k: 0.30, maxPages: Infinity },
] as const;

function formatCurrency(n: number): string {
  if (n >= 1_000_000) return `$${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `$${(n / 1_000).toFixed(1)}K`;
  return `$${n.toFixed(0)}`;
}

function formatNumber(n: number): string {
  return n.toLocaleString('en-US');
}

function bestWraithTier(pages: number) {
  let bestIdx = 0;
  let bestCost = Infinity;

  for (let i = 0; i < WRAITH_TIERS.length; i++) {
    const tier = WRAITH_TIERS[i];
    if (pages > tier.maxPages) continue;
    const overage = Math.max(0, pages - tier.includedPages);
    const cost = tier.monthlyBase + (overage / 1000) * tier.overagePer1k;
    if (cost < bestCost) {
      bestCost = cost;
      bestIdx = i;
    }
  }

  const best = WRAITH_TIERS[bestIdx];
  const overage = Math.max(0, pages - best.includedPages);
  const totalCost = best.monthlyBase + (overage / 1000) * best.overagePer1k;
  return { tier: { name: best.name, monthlyBase: best.monthlyBase }, monthlyCost: totalCost };
}

export function ROICalculator() {
  const [pages, setPages] = useState(50_000);
  const [solutionIdx, setSolutionIdx] = useState(0);

  const solution = CURRENT_SOLUTIONS[solutionIdx];
  const currentCost = (pages / 1000) * solution.costPer1k;
  const wraith = bestWraithTier(pages);
  const savings = currentCost - wraith.monthlyCost;
  const savingsPercent = currentCost > 0 ? (savings / currentCost) * 100 : 0;
  const yearlySavings = savings * 12;

  const pageOptions = [1_000, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000, 5_000_000];
  const closestIdx = pageOptions.reduce((best, val, i) =>
    Math.abs(val - pages) < Math.abs(pageOptions[best] - pages) ? i : best, 0
  );

  return (
    <div className="space-y-6">
      {/* Inputs */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div>
          <label className="block text-sm font-medium text-zinc-400 mb-2">
            Current solution
          </label>
          <select
            value={solutionIdx}
            onChange={(e) => setSolutionIdx(Number(e.target.value))}
            className="w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 focus:outline-none focus:ring-1 focus:ring-emerald-500"
          >
            {CURRENT_SOLUTIONS.map((s, i) => (
              <option key={s.name} value={i}>{s.name} (~${s.costPer1k}/1K pages)</option>
            ))}
          </select>
        </div>

        <div>
          <label className="block text-sm font-medium text-zinc-400 mb-2">
            Monthly page volume: {formatNumber(pages)}
          </label>
          <input
            type="range"
            min={0}
            max={pageOptions.length - 1}
            step={1}
            value={closestIdx}
            onChange={(e) => setPages(pageOptions[Number(e.target.value)])}
            className="w-full accent-emerald-500"
          />
          <div className="flex justify-between text-xs text-zinc-500 mt-1">
            <span>1K</span>
            <span>5M</span>
          </div>
        </div>
      </div>

      {/* Results */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
          <div className="text-xs text-zinc-500 uppercase tracking-wide">Current cost</div>
          <div className="text-2xl font-bold text-zinc-100 mt-1">{formatCurrency(currentCost)}<span className="text-sm font-normal text-zinc-400">/mo</span></div>
          <div className="text-xs text-zinc-500 mt-1">{solution.name}</div>
        </div>

        <div className="rounded-lg border border-emerald-800 bg-emerald-950/30 p-4">
          <div className="text-xs text-emerald-400 uppercase tracking-wide">Wraith cost</div>
          <div className="text-2xl font-bold text-emerald-300 mt-1">{formatCurrency(wraith.monthlyCost)}<span className="text-sm font-normal text-emerald-500">/mo</span></div>
          <div className="text-xs text-zinc-500 mt-1">{wraith.tier.name} tier</div>
        </div>

        <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
          <div className="text-xs text-zinc-500 uppercase tracking-wide">
            {savings > 0 ? 'You save' : 'Difference'}
          </div>
          <div className={`text-2xl font-bold mt-1 ${savings > 0 ? 'text-emerald-300' : 'text-zinc-400'}`}>
            {savings > 0 ? formatCurrency(savings) : `-${formatCurrency(Math.abs(savings))}`}
            <span className="text-sm font-normal text-zinc-400">/mo</span>
          </div>
          {savings > 0 && (
            <div className="text-xs text-emerald-500 mt-1">
              {savingsPercent.toFixed(0)}% less &middot; {formatCurrency(yearlySavings)}/yr
            </div>
          )}
        </div>
      </div>

      {/* Comparison table */}
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-zinc-700">
              <th className="text-left py-2 text-zinc-400 font-medium">Volume</th>
              <th className="text-right py-2 text-zinc-400 font-medium">{solution.name}</th>
              <th className="text-right py-2 text-emerald-400 font-medium">Wraith</th>
              <th className="text-right py-2 text-zinc-400 font-medium">Savings</th>
            </tr>
          </thead>
          <tbody>
            {[10_000, 100_000, 500_000, 1_000_000].map((vol) => {
              const otherCost = (vol / 1000) * solution.costPer1k;
              const w = bestWraithTier(vol);
              const s = otherCost - w.monthlyCost;
              return (
                <tr key={vol} className="border-b border-zinc-800">
                  <td className="py-2 text-zinc-300">{formatNumber(vol)} pages</td>
                  <td className="py-2 text-right text-zinc-300">{formatCurrency(otherCost)}</td>
                  <td className="py-2 text-right text-emerald-300">{formatCurrency(w.monthlyCost)}</td>
                  <td className={`py-2 text-right ${s > 0 ? 'text-emerald-400' : 'text-zinc-500'}`}>
                    {s > 0 ? `${formatCurrency(s)} (${((s / otherCost) * 100).toFixed(0)}%)` : '-'}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <p className="text-xs text-zinc-500">
        Estimates based on published pricing. Self-hosted costs include compute
        (Playwright/Puppeteer assume ~6 sessions per 16GB VM at $80/mo). Wraith
        self-hosted runs 50-100+ sessions on a single VM. Actual costs vary by usage pattern.
      </p>
    </div>
  );
}
