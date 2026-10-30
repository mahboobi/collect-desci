// Rewards at the screen boundary: the offer is shown, approvals are asked
// for, and Pay stays locked until everything the creator must check is true.
// Who gets paid is decided and tested in Rust; this checks the plumbing.
import QtQuick
import QtTest
import collect
import "../src/qml/screens"

Item {
    id: host
    width: 1100
    height: 900

    readonly property var experiment: ({
        id: "q7k2m3x4w3b5n6tz", title: "Birds of Dhun", description: "", protocols: [],
        starts_at: "2026-10-20T00:00:00Z", ends_at: "2026-11-20T00:00:00Z",
        creator: { key: "aa", name: "" },
        reward: { token: "lez:testnet", amount: "500000", max_per_submitter: 2 }
    })
    readonly property double before: Date.parse("2026-11-01T00:00:00Z")
    readonly property double after: Date.parse("2026-11-21T00:00:00Z")

    readonly property var plan: ({
        reward: { amount: "0.0005", amount_lepta: "500000", max_per_submitter: 2, symbol: "TestToken" },
        ended: true, pending: 1, total: "0.001", total_lepta: "1000000",
        payees: [ { submitter: "a1", name: "alice", wallet: "1111111111111111111111111111111Gp",
                    submissions: ["logos:z1", "logos:z2"], count: 2, amount: "0.001", amount_lepta: "1000000" } ],
        skipped: [ { submitter: "b2", name: "bob", count: 1, reason: "did not include a wallet address" } ],
        from: { address: "SENDER", balance: "1", covers: true },
        payouts: []
    })

    Component { id: joinComponent; Join {} }
    Component { id: resultsComponent; Results {} }
    Component { id: rewardsComponent; Rewards { experiment: host.experiment } }

    TestCase {
        name: "Rewards"
        when: windowShown

        function find(item, name) {
            if (item.objectName === name)
                return item;
            for (var i = 0; i < item.children.length; i++) {
                var hit = find(item.children[i], name);
                if (hit)
                    return hit;
            }
            if (item.contentItem && item.contentItem !== item) {
                var inner = find(item.contentItem, name);
                if (inner)
                    return inner;
            }
            return null;
        }

        function test_lepta_format_exactly() {
            compare(Core.formatLepta("500000"), "0.0005");
            compare(Core.formatLepta("1500000000"), "1.5");
            compare(Core.formatLepta("2000000000"), "2");
            compare(Core.formatLepta("18446744073709551615"), "18446744073.709551615");
            compare(Core.formatLepta(""), "0");
        }

        function test_a_submitter_sees_the_offer_and_what_to_do() {
            var j = joinComponent.createObject(host, { width: 900, height: 800, experiment: host.experiment,
                                                       mine: false, nowMs: host.before });
            var terms = find(j, "rewardTerms").text;
            verify(terms.indexOf("0.0005 TestToken") === 0, terms);
            verify(terms.indexOf("include your wallet address") > 0, terms);
            verify(!find(j, "saveReward").parent.visible, "a submitter was offered the creator's controls");
            verify(!find(j, "payRewards").visible);
            j.destroy();
        }

        function test_pay_appears_for_the_creator_only_after_the_end() {
            var j = joinComponent.createObject(host, { width: 900, height: 800, experiment: host.experiment,
                                                       mine: true, nowMs: host.before });
            verify(!find(j, "payRewards").visible, "pay offered before the end date");
            verify(find(j, "saveReward").parent.visible);
            j.nowMs = host.after;
            verify(find(j, "payRewards").visible);
            verify(!find(j, "saveReward").parent.visible, "the terms were editable after the end");
            j.destroy();
        }

        function test_review_buttons_approve_and_undo() {
            var row = { cid: "logos:z1", submitter: "a1", submitter_name: "alice", version: 1,
                        submitted_at: "2026-10-25T09:00:00Z", values: {}, approval: "pending", paid: false };
            var r = resultsComponent.createObject(host, { width: 1000, height: 700, results: {
                columns: [], rows: [row], hidden_keys: [], summary: {} } });
            var asked = [];
            r.setApproval.connect(function (cid, status) { asked.push(cid + ":" + status); });
            waitForRendering(r);
            mouseClick(find(r, "review_approved"));
            compare(asked, ["logos:z1:approved"]);

            row.approval = "approved";
            r.results = { columns: [], rows: [row], hidden_keys: [], summary: {} };
            waitForRendering(r);
            mouseClick(find(r, "review_approved"));
            compare(asked[1], "logos:z1:pending", "pressing the chosen verdict again did not undo it");
            r.destroy();
        }

        function test_push_needs_an_approval_and_reports_what_happened() {
            var row = { cid: "logos:z1", submitter: "a1", submitter_name: "alice", version: 1,
                        submitted_at: "2026-10-25T09:00:00Z", values: {}, approval: "pending", paid: false };
            var r = resultsComponent.createObject(host, { width: 1000, height: 700, results: {
                columns: [], rows: [row], hidden_keys: [], summary: {} } });
            var push = find(r, "pushButton");
            verify(!push.enabled, "push offered with nothing approved");
            verify(find(r, "pushExplain").text.indexOf("Approve") === 0);

            row.approval = "approved";
            r.results = { columns: [], rows: [row], hidden_keys: [], summary: {} };
            verify(push.enabled);
            var pushed = 0;
            r.pushToStorage.connect(function () { pushed++; });
            waitForRendering(r);
            mouseClick(push);
            compare(pushed, 1);

            r.pushResult = { ok: true, data: { cid: "logos:zRes", rows: 1, media: { named: 2, held: 1, missing: ["logos:zP"] },
                                               listed_in_manifest: true } };
            var said = find(r, "pushOutcome").text;
            verify(said.indexOf("logos:zRes") > 0, said);
            verify(said.indexOf("1 of 2") > 0, "a missing photo was not mentioned: " + said);

            r.pushing = true;
            verify(!push.enabled, "a second push could start during the first");
            r.destroy();
        }

        function test_pay_stays_locked_until_confirmed_and_checked() {
            var w = rewardsComponent.createObject(host, { width: 1000, height: 850, plan: host.plan,
                                                          fromAddress: "SENDER" });
            var pay = find(w, "payButton");
            verify(!pay.enabled, "pay enabled before the box was ticked");
            verify(find(w, "pendingWarning").visible, "unreviewed submissions were not mentioned");

            find(w, "confirmPay").toggled(true);
            verify(pay.enabled);

            // A different address than the one whose balance was checked.
            w.typedFrom = "SOMEONE-ELSE";
            verify(!pay.enabled, "pay enabled for an account whose balance was not checked");
            w.typedFrom = "SENDER";

            var sent = [];
            w.payAll.connect(function (from) { sent.push(from); });
            waitForRendering(w);
            mouseClick(pay);
            compare(sent, ["SENDER"]);
            verify(!find(w, "confirmPay").on, "the confirmation stayed ticked for another run");
            w.destroy();
        }

        function test_an_unaffordable_total_cannot_be_paid() {
            var poor = JSON.parse(JSON.stringify(host.plan));
            poor.from.covers = false;
            var w = rewardsComponent.createObject(host, { width: 1000, height: 850, plan: poor, fromAddress: "SENDER" });
            find(w, "confirmPay").toggled(true);
            verify(!find(w, "payButton").enabled);
            w.destroy();
        }
    }
}
