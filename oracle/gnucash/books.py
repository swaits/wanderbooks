"""Build books in GnuCash and report its balances.

stdin: a JSON list of books:
  {"accounts": [{"id": 1, "code": "1000", "name": "Cash", "type": "Asset"}],
   "entries":  [{"date": "2026-01-15", "description": "...",
                 "lines": [{"account": 1, "side": "debit", "amount": "12.34"}]}],
   "as_of":    ["2026-01-31", ...]}
stdout: a JSON list, one per book:
  {"balances": {"2026-01-31": {"1": "12.34", ...}, ...}}
Balances are GnuCash's own: the sum of split values on the account through
the end of that day, debits positive, as an exact decimal string.
"""
import datetime, json, os, sys, tempfile
from decimal import Decimal
import gnucash
from gnucash import Account, GncNumeric, Session, SessionOpenMode, Split, Transaction

TYPES = {
    "Asset": gnucash.ACCT_TYPE_ASSET,
    "Liability": gnucash.ACCT_TYPE_LIABILITY,
    "Equity": gnucash.ACCT_TYPE_EQUITY,
    "Revenue": gnucash.ACCT_TYPE_INCOME,
    "Expense": gnucash.ACCT_TYPE_EXPENSE,
}


def numeric(amount: str) -> GncNumeric:
    d = Decimal(amount)
    sign, digits, exp = d.as_tuple()
    denom = 10 ** max(0, -exp)
    num = int(d * denom)
    return GncNumeric(num, denom)


def decimal_of(n) -> str:
    return str(Decimal(n.num()) / Decimal(n.denom()))


def end_of_day(date: str) -> int:
    d = datetime.date.fromisoformat(date)
    return int(datetime.datetime(d.year, d.month, d.day, 23, 59, 59,
                                 tzinfo=datetime.timezone.utc).timestamp())


def run(book_spec, path):
    s = Session(f"xml://{path}", SessionOpenMode.SESSION_NEW_STORE)
    try:
        book = s.book
        root = book.get_root_account()
        usd = book.get_table().lookup("CURRENCY", "USD")
        accounts = {}
        for a in book_spec["accounts"]:
            acct = Account(book)
            acct.SetName(f'{a["code"]} {a["name"]}')
            acct.SetCode(a["code"])
            acct.SetType(TYPES[a["type"]])
            acct.SetCommodity(usd)
            root.append_child(acct)
            accounts[a["id"]] = acct
        for e in book_spec["entries"]:
            d = datetime.date.fromisoformat(e["date"])
            tx = Transaction(book)
            tx.BeginEdit()
            tx.SetCurrency(usd)
            tx.SetDate(d.day, d.month, d.year)
            tx.SetDescription(e.get("description", ""))
            for line in e["lines"]:
                value = numeric(line["amount"])
                if line["side"] == "credit":
                    value = value.neg()
                sp = Split(book)
                sp.SetParent(tx)
                sp.SetAccount(accounts[line["account"]])
                sp.SetValue(value)
                sp.SetAmount(value)
            tx.CommitEdit()
        out = {}
        for date in book_spec["as_of"]:
            t = end_of_day(date)
            out[date] = {str(i): decimal_of(a.GetBalanceAsOfDate(t)) for i, a in accounts.items()}
        return {"balances": out}
    finally:
        s.end()


def main():
    books = json.load(sys.stdin)
    results = []
    with tempfile.TemporaryDirectory() as tmp:
        for i, b in enumerate(books):
            results.append(run(b, os.path.join(tmp, f"book{i}.gnucash")))
    json.dump(results, sys.stdout)


if __name__ == "__main__":
    main()
