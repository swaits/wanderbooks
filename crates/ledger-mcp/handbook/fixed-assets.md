# Fixed assets

- Register with books.register_asset, inspect with books.asset_schedule and books.list_assets, post monthly depreciation with books.run_depreciation (exempt from the receipt policy; the schedule is the evidence), and dispose with books.dispose_asset (preview first; the bill of sale is the receipt).
- Depreciation is straight-line, book basis only; tax depreciation stays with the CPA.
- Every asset gets a tag for the label on the thing itself (books.register_asset generates the next one, like FA-0001, unless the human has their own); record the manufacturer's serial number too, change either with books.label_asset, and print label sheets with books.asset_labels (give the human the PDF; print at 100% on Avery 5160/8160). Asset tools take the tag in place of the id.
- Record every asset's federal tax basis beside its book value (books.set_asset_tax_basis, or tax_basis_rule on books.register_asset): cost for a purchase; for a member's contribution, the member's adjusted basis (what they paid, less depreciation they took) with rule personal_use_conversion (their personal property: tax basis is the lesser of value and their basis) or carryover (their business property). Book value stays fair market value.
- books.export fixed_asset_register is the register for the CPA; books.show asset opens one in the viewer.
