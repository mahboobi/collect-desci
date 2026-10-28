"""Validate Collect example documents and announcements against schemas/collect.schema.json.

Usage: python3 tools/validate.py   (needs: pip install jsonschema)
"""
import json, pathlib, sys
from jsonschema import Draft202012Validator, FormatChecker

root = pathlib.Path(__file__).resolve().parent.parent
schema = json.loads((root / "schemas/collect.schema.json").read_text())
Draft202012Validator.check_schema(schema)
v = Draft202012Validator(schema, format_checker=FormatChecker())

def load(name):
    return json.loads((root / "examples" / name).read_text())

docs = [load("experiment.json"), load("protocol.json"), load("submission.json"), load("index.json")] + load("announcements.json")
fail = 0
for d in docs:
    label = d.get("schema") or ("announcement:" + d.get("kind", "?"))
    errs = list(v.iter_errors(d))
    if errs:
        fail += 1
        print("FAIL", label)
        for e in errs[:4]:
            print("    ", list(e.absolute_path), e.message[:200])
    else:
        print("ok  ", label)

def mutate(base, fn):
    d = json.loads(json.dumps(base)); fn(d); return d

neg = [
    ("choice field without options", mutate(load("protocol.json"), lambda d: d["fields"][1].pop("config"))),
    ("location field without store rule", mutate(load("protocol.json"), lambda d: d["fields"][4]["config"].pop("store"))),
    ("unknown field type", mutate(load("protocol.json"), lambda d: d["fields"][0].__setitem__("type", "audio"))),
    ("submission with bad field key", mutate(load("submission.json"), lambda d: d["values"].__setitem__("Bad Key", "x"))),
    ("submission without signature", mutate(load("submission.json"), lambda d: d.pop("sig"))),
    ("photo with unsupported type", mutate(load("submission.json"), lambda d: d["values"]["photos"][0].__setitem__("mime", "image/gif"))),
    ("index page over 1000 entries", mutate(load("index.json"), lambda d: d.__setitem__("entries", d["entries"] * 501))),
    ("receipt missing cid", mutate(load("announcements.json")[2], lambda d: d.pop("cid"))),
    ("head with extra field", mutate(load("announcements.json")[0], lambda d: d.__setitem__("cid", "logos:x"))),
]
for label, d in neg:
    if v.is_valid(d):
        fail += 1; print("FAIL negative accepted:", label)
    else:
        print("ok   negative rejected:", label)

print("\nALL PASSED" if not fail else f"\n{fail} FAILURE(S)")
sys.exit(1 if fail else 0)
