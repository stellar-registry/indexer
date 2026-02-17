import * as cdk from 'aws-cdk-lib';
import * as apigateway from 'aws-cdk-lib/aws-apigateway';
import * as lambda from 'aws-cdk-lib/aws-lambda-nodejs';
import { Construct } from 'constructs';
import * as events from 'aws-cdk-lib/aws-events';
import * as targets from 'aws-cdk-lib/aws-events-targets';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';

export class GoldskyStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    const httpHandler = new lambda.NodejsFunction(this, 'goldsky', {
      entry: 'lib/get-contracts.ts',
      handler: 'handler',
    });

    const periodicLambda = new lambda.NodejsFunction(this, 'periodic-lambda', {
      entry: 'lib/periodic-lambda.ts',
      handler: 'handler'
    })

    const rule = new events.Rule(this, 'ScheduleRule', {
      schedule: events.Schedule.rate(cdk.Duration.minutes(1)),
    });

    rule.addTarget(new targets.LambdaFunction(periodicLambda));

    const dbSecret = secretsmanager.Secret.fromSecretNameV2(this, 'dbSecret', 'goldsky-pg-url');

    // Grant your Lambda access
    dbSecret.grantRead(periodicLambda);
    dbSecret.grantRead(httpHandler);

    const goldsky_api = new apigateway.RestApi(this, 'api', {
      defaultCorsPreflightOptions: {
        allowOrigins: ['*'],
        allowMethods: ['ANY'],
      },
      deployOptions: {
        throttlingBurstLimit: 1,
        throttlingRateLimit: 1,
      }
    })

    new cdk.CfnOutput(this, 'apiUrl', { value: goldsky_api.url });

    const contracts = goldsky_api.root.addResource('registry').addResource('contracts')

    contracts.addMethod(
      'GET',
      new apigateway.LambdaIntegration(httpHandler, { proxy: true,  timeout: cdk.Duration.seconds(5) }),
      {
        requestParameters: {
          // Optional http query parameters 
          'method.request.querystring.limit': false,
          'method.request.querystring.cursor': false,
        },
      }
    )
  }
}
