## AWS Lambda initial setup
0. Make sure docker is installed and daemon is running
1. Install [AWS CDK](https://docs.aws.amazon.com/cdk/v2/guide/getting-started.html) and [AWS cli](https://aws.amazon.com/cli/) (important: make sure AWS-cli is v2)
2. Create AWS account (if not created yet)
3. Select region. In this example we will be using `us-east-2`
4. Run `aws configure`. To get the keys go to your AWS console -> click on the account (top right corner) -> Security credentials -> Access Keys -> create a key. Copy-paste key ID and secret in the terminal. Note: this will create root key that can access anything!
5. Run `cdk bootstrap`. You can read more about CDK bootstrap [here](https://docs.aws.amazon.com/cdk/v2/guide/bootstrapping.html)
6. 
