## AWS Lambda initial setup
0. Make sure docker is installed and daemon is running
1. Install [AWS CDK](https://docs.aws.amazon.com/cdk/v2/guide/getting-started.html) and [AWS cli](https://aws.amazon.com/cli/) (important: make sure AWS-cli is v2)
2. Create AWS account (if not created yet)
3. Select region. In this example we will be using `us-east-2`
4. Run `aws configure`. To get the keys go to your AWS console -> click on the account (top right corner) -> Security credentials -> Access Keys -> create a key. Copy-paste key ID and secret in the terminal. Note: this will create root key that can access anything!
5. Run `cdk bootstrap`. You can read more about CDK bootstrap [here](https://docs.aws.amazon.com/cdk/v2/guide/bootstrapping.html)
6. Create secrets for PG database: `aws secretsmanager create-secret --name goldsky-pg-url --secret-string "test"` (replace "test" with actual PG DB url) 
7. Run `cdk deploy` to deploy and redeploy. (On subsequent runs you can skip previous steps, just make sure correct AWS profile is being used 
(should be named `default`: check `aws configure list` output, as well as `AWS_DEFAULT_PROFILE` and `AWS_PROFILE` env variables)

## Migrations and table structure 
To create tables run psql <postgres url> -f sql/init.sql
Project has a simplified tables versioning, where current table version is set in `lib/db.types.ts` 
(for example, `deploys_4` uses version 4 of deploys table)
The `sql/init.sql` script in turn will drop version 2 of the table, keep version 3 
(so runtime lambdas may continue work with it), and create version 4.
The upgrade flow should be the following:
1. Create new version of DB (for example, adding new row), in this example v4
2. Make necessary code changes in periodic lambda. If necessary, keep versioning of table for HTTP lambda to previous one (in this example, v3)  
3. Run `psql -f sql/init.sql`
4. Run `cdk deploy` -> periodic lambda will start populating new table
5. If necessary, rollback to previous stable version (in this example version 3), drop v4 table and repeat the process
Note: dropping tables created by lambda is safe (and adviced), dropping tables managed by goldsky (`events` table), is strongly not recommended! 
