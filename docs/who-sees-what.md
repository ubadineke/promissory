# Who sees what

The short version of [`privacy-review.md`](privacy-review.md).

| Party | Sees | Doesn't see |
|---|---|---|
| **The chain** (everyone) | Ordinary shielded Ironwood transactions. If you were offline in the hour before a payment, possibly that it was pre-signed (an old anchor). | Who paid whom, or how much. |
| **The payee** | Amount, schedule, memo, and whether each payment arrived (inherent to receiving). With proof of reservation: that the next payments are funded, and a warning if you cancel. | Your other funds and orders, or where a cancel sent the money. |
| **The keeper** | Encrypted blobs until each due time; then the transaction, which is about to be public anyway. | Your keys, the amount, the recipient, or who you are. It can't pay early or redirect. |
| **drand** | That someone polls for beacons. | Anything about your payments: encryption is done offline. |
| **lightwalletd** | Your IP while syncing (as for any wallet), and the keeper's IP when it broadcasts. `--tor` hides both. | Transaction contents. |
| **The rate source** | That someone asked for ZEC/NGN, and only after you turn on Send home. | Anything about payments. |
