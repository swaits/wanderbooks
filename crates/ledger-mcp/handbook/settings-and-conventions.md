# Settings

Until settings move into the books themselves (planned next), they live in the settings file books.settings names (books_toml), a TOML file; this section changes then.

- That file holds the company's facts and settings (name, fiscal year, tax form, members and their capital accounts, basis, receipt threshold, cash flow roles, close target). books.settings shows what is in force and lists problems: settings that disagree with the chart.
- Change it when the human tells you a company fact, when a tool refuses naming a setting, when books.settings or books.digest lists a settings problem, and after books.renumber_account lists config_mentions.
- The server reloads it whenever it changes, with no restart: after each change, call books.settings and check reload_error is null (a broken edit leaves the last good settings in force) and problems is empty.
- Never put a member's SSN or any TIN in it.
