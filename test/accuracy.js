import http from 'k6/http';
import { SharedArray } from 'k6/data';
import { Counter } from 'k6/metrics';
import exec from 'k6/execution';

const testData = new SharedArray('test-data', function () {
    return JSON.parse(open('./test-data.json')).entries;
});

const fpCount = new Counter('fp_count');
const fnCount = new Counter('fn_count');
const tpCount = new Counter('tp_count');
const tnCount = new Counter('tn_count');
const errorCount = new Counter('error_count');

export const options = {
    summaryTrendStats: ['p(99)'],
    scenarios: {
        accuracy: {
            executor: 'shared-iterations',
            vus: 20,
            iterations: testData.length,
            maxDuration: '30m',
        },
    },
};

export default function () {
    const idx = exec.scenario.iterationInTest;
    const entry = testData[idx];
    const base = __ENV.BASE_URL || 'http://127.0.0.1:9999';
    const res = http.post(
        `${base}/fraud-score`,
        JSON.stringify(entry.request),
        { headers: { 'Content-Type': 'application/json' }, timeout: '5s' }
    );

    if (res.status !== 200) {
        errorCount.add(1);
        return;
    }

    const body = JSON.parse(res.body);
    const expectedApproved = entry.expected_approved;
    if (expectedApproved === body.approved) {
        if (body.approved) tnCount.add(1);
        else tpCount.add(1);
    } else if (body.approved) {
        fnCount.add(1);
    } else {
        fpCount.add(1);
    }
}

export function handleSummary(data) {
    const fp = data.metrics.fp_count ? data.metrics.fp_count.values.count : 0;
    const fn = data.metrics.fn_count ? data.metrics.fn_count.values.count : 0;
    const errs = data.metrics.error_count ? data.metrics.error_count.values.count : 0;
    const p99 = data.metrics.http_req_duration.values['p(99)'];
    console.log(`FP=${fp} FN=${fn} errors=${errs} p99=${p99.toFixed(2)}ms`);
    return {};
}
