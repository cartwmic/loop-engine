#!/usr/bin/env python3
"""Explicit current-generation adapter. No historical fields synthesized."""
import argparse,ast,hashlib,json,re
from pathlib import Path
ENGINE_SHA="e1599f645f93d0b3a8d64403253f74e0d3e956145b80aa791f57cc3f752723bc"
def need(v,m):
    if not v: raise RuntimeError(m)
def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def load(p): return json.loads(Path(p).read_bytes())
def inventory(root): return {str(p.relative_to(root)):sha(p) for p in sorted(root.rglob("*")) if p.is_file()}
def digest(v): return hashlib.sha256(json.dumps(v,sort_keys=True,separators=(",",":")).encode()).hexdigest()
def validate(root,receipt_path,binding_path,repair_path,evidence_path,scripted=False):
    root=Path(root).resolve(strict=True); receipt_path=Path(receipt_path).resolve(strict=True); out=receipt_path.parent
    b=load(binding_path); r=load(receipt_path); linked=load(repair_path); identity=load(evidence_path)
    mode="scripted" if scripted else "native"
    for obj in [b,r,linked,identity]:
        need(obj["generation"]=="new-current-01" and obj["backend"]==mode,"generation/backend mismatch")
    need(r["status"]==("scripted-complete" if scripted else "passed"),"current proof incomplete")
    need(r["mechanical_only"] is True and r["semantic_review"] is False,"semantic judgment claim")
    need(b["runtime_sha256"]==sha(receipt_path) and b["identity_sha256"]==sha(evidence_path) and b["linked_sha256"]==sha(repair_path),"receipt identity mismatch")
    src=inventory(root)
    need(r["source_before"]==r["source_after"]==src and b["source_sha256"]==digest(src),"current source mismatch")
    need(b["engine_sha256"]==r["engine_sha256"]==ENGINE_SHA,"frozen engine mismatch")
    provider=Path(r["provider_path"]).resolve(strict=True)
    need(provider.is_relative_to(out/"build") and sha(provider)==b["provider_sha256"]==r["provider_sha256"],"provider identity mismatch")
    index=load(root/"implementation-evidence/source-inventory.json")
    need(index["files_sha256"]=={k:v for k,v in src.items() if Path(k).name!="source-inventory.json"},"source inventory mismatch")
    mapping=load(root/"implementation-evidence/four-acceptance-proof-map.json")
    need([v["requirement"] for v in mapping]==[
        "A caller can inspect the frozen policies and artifact schemas before evaluating a transition.",
        "A malformed subject artifact is denied with each violated structural rule named.",
        "A checked transition refuses stale, self-authored, duplicate-author, or configured-obligation-incomplete evidence.",
        "The reference workflow reaches its terminal state only after its configured validation obligations are satisfied."], "four exact rows")
    matrix=(root/"implementation-evidence/requirement-to-proof.md").read_text(); refs=set()
    for row in mapping:
        lines=[l.split("|") for l in matrix.splitlines() if l.startswith("| "+row["requirement"]+" |")]
        need(len(lines)==1 and re.findall(chr(96)+"([^"+chr(96)+"]+)"+chr(96),lines[0][2])==row["proofs"],"matrix proof mismatch")
        refs.update(row["proofs"])
    need(refs==set(index["proof_inventory"]),"proof names mismatch")
    for ref in refs:
        fn,name=ref.split("::",1); p=root/fn; text=p.read_text()
        if p.suffix==".rs": need(re.search(r"fn\s+"+re.escape(name)+r"\s*\(",text),"Rust proof absent")
        else: need(any(isinstance(n,ast.FunctionDef) and n.name==name for n in ast.walk(ast.parse(text))),"Python proof absent")
    need(r["protocol"]==["describe-exact","opaque-describe-exact","unexpected-envelope-refused","unknown-operation-refused","data-dump-exact"],"protocol controls incomplete")
    need([j["profile"] for j in r["journeys"]]==["minimal","standard","high-rigor"],"profiles incomplete/duplicate")
    for j in r["journeys"]:
        f=j["final_show"]; submitted=load(root/("provider/data/configs/"+j["profile"]+".json"))
        need({k:v for k,v in f["initial_input"].items() if k!="artifact_root"}==submitted and set(f["initial_input"])==set(submitted)|{"artifact_root"},"frozen input mismatch")
        need(Path(f["initial_input"]["artifact_root"]).resolve().is_relative_to(out),"artifact root escaped")
        need(f["current_state"]=="end" and f["lifecycle"]=="final" and f["completion_mode"]=="completed" and
             f["has_overrides"] is False and f["override_count"]==0 and f["requestable_events"]==[],"terminal/override mismatch")
        history=j["history"]; seq=[x["sequence"] for x in history]
        need(history and seq==sorted(set(seq)) and any(x["action"]["kind"]=="transition" and x["action"]["transition"]["target"]=="end"
             and x["action"]["outcome"]["outcome"]=="committed" for x in history),"terminal history incomplete")
        required={"stale-revision","stale-config","subject-author"}
        if submitted["review_policies"].get("intent-review"): required.add("standing-fail-not-outvoted")
        if any(a.get("required_authors",1)>1 for axes in submitted["review_policies"].values() for a in axes): required.add("duplicate-author")
        need(required<={x["case"] for x in j["negative_cases"]},"negative outcome controls incomplete")
    names=["valid-control","stale-with-sufficient","stale-noncounting","stale-malformed-order-unchanged","standing-fail-and-own-restoration"]
    names += ["identity-"+f+"-"+s for f in ["subject_revision","config_version"] for s in ["missing","non-string","blank"]]
    need([c["case"] for c in identity["cases"]]==names,"identity case set differs")
    for c in identity["cases"]:
        expected="completed" if c["case"] in ["valid-control","stale-with-sufficient","stale-malformed-order-unchanged"] else "rejected"
        need(c["expected"]==c["event"]["status"]==expected,"identity outcome differs")
        if expected=="rejected":
            need(c["before"]["context"]==c["after"]["context"] and c["before"]["current_state"]==c["after"]["current_state"],"refusal mutated state/context")
            need(c["restoration"]["status"]=="completed","conforming restoration absent")
    a=load(out/"final-accounting.json")
    need(b["accounting_sha256"]==r["accounting_sha256"]==sha(out/"final-accounting.json"),"accounting identity mismatch")
    need(a["backend"]==mode and a["success"] is True and a["all_children_waited"] is True and a["residual_groups"]==[],"unreconciled process outcome")
    commands=a["commands"]; need(commands and len(commands)<=1200 and a["elapsed_seconds"]<=3600,"process/wall limits")
    for i,row in enumerate(commands,1):
        need(row["index"]==i and row["backend"]==mode and row["status"]=="completed" and row["waited"] is True and
             row["residual_group"] is False and type(row["exit_code"]) is int and row["exit_code"]==row["expected_exit"],"command failed/incomplete")
        d=out/"commands"/("%04d"%i); need(load(d/"receipt.json")==row,"command record differs")
        for name in ["stdin","stdout","stderr"]: need(row[name+"_sha256"]==sha(d/(name+".txt")),"stream tampered")
        need(row["environment_limits"]=={"CARGO_BUILD_JOBS":"2","RUST_TEST_THREADS":"2","RAYON_NUM_THREADS":"2","OMP_NUM_THREADS":"2"},"thread limits differ")
    def observed(index):
        need(type(index) is int and 1<=index<=len(commands),"invalid observation index")
        return load(out/"commands"/("%04d"%index)/"stdout.txt")
    for j in r["journeys"]:
        need(observed(j["final_show_index"])["result"]==j["final_show"] and observed(j["history_index"])["result"]==j["history"],"terminal summary not actual captured output")
    for c in identity["cases"]:
        for field in ["event","restoration"]:
            v=c[field]
            if v is not None:
                need(observed(v["_command_index"])=={k:x for k,x in v.items() if k!="_command_index"},"identity summary not actual captured output")
    need([c["case"] for c in linked["cases"]]==["missing","mismatch","escape","valid"],"linked case set differs")
    for c in linked["cases"]:
        i=c["command_index"]; row=commands[i-1]; d=out/"commands"/("%04d"%i)
        need(row["argv"]==[str(provider)],"linked proof wrong invocation")
        if c["case"]=="escape":
            need(row["exit_code"]==1 and (d/"stdout.txt").read_bytes()==b"" and "artifact intent.json escapes configured root" in (d/"stderr.txt").read_text(),"containment taxonomy")
        else:
            v=load(d/"stdout.txt"); need(row["exit_code"]==0,"linked command failed")
            if c["case"]=="valid": need(v=={"result":"allow"},"valid parent denied")
            else: need(v["result"]=="deny" and any(x["rule"]=="revision-link" and x["subject"]=="design.json" and x["target"]=="intent.json" for x in v["feedback"]["details"]),"missing/mismatch taxonomy")
    return {"status":"SCRIPTED_ADAPTER_PATH_ONLY" if scripted else "GENUINE_CURRENT_MECHANICAL_PROOF","native_proof":not scripted,"semantic_review":False,"commands":len(commands)}
def main():
    p=argparse.ArgumentParser(); p.add_argument("--source",type=Path,required=True)
    for name in ["receipt","binding","repair-proof","evidence-proof"]: p.add_argument("--"+name,type=Path,required=True)
    p.add_argument("--scripted-test-only",action="store_true"); a=p.parse_args()
    print(json.dumps(validate(a.source,a.receipt,a.binding,a.repair_proof,a.evidence_proof,a.scripted_test_only)))
if __name__=="__main__": main()
