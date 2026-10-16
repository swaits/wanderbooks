# Revenue and payouts

- Platform payouts (app stores, payment processors, marketplaces): never book the net deposit as revenue. books.record_payout grosses it up (gross revenue, fees, adjustments) into a draft and refuses unless it reconciles to the cent; attach the payout statement, then preview and post.
- Invoices and bills: books.create_invoice, books.receive_payment, books.create_bill and books.pay_bill post to the control accounts (the control-roles section).
