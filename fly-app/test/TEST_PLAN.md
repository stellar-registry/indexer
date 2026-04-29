We are testing manually for now 😞

Check all these endpoints in production...

- https://registry-indexer.fly.dev/
- https://registry-indexer.fly.dev/v1
- https://registry-indexer.fly.dev/v1/registries
- https://registry-indexer.fly.dev/v1/wasms
- https://registry-indexer.fly.dev/v1/wasms/registry
- https://registry-indexer.fly.dev/v1/wasms/unverified/guess-the-number
- https://registry-indexer.fly.dev/v1/wasms/unverified/guess-the-number/v/0.0.1
- https://registry-indexer.fly.dev/v1/contracts
- https://registry-indexer.fly.dev/v1/contracts/unverified

...and compare them to what you see in local (`cd` to `fly-app` and run `PORT=4444 cargo run`):

- http://localhost:4444/
- http://localhost:4444/v1
- http://localhost:4444/v1/registries
- http://localhost:4444/v1/wasms
- http://localhost:4444/v1/wasms/registry
- http://localhost:4444/v1/wasms/unverified/guess-the-number
- http://localhost:4444/v1/wasms/unverified/guess-the-number/v/0.0.1
- http://localhost:4444/v1/contracts
- http://localhost:4444/v1/contracts/unverified
