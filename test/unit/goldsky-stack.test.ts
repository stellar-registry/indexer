import * as cdk from 'aws-cdk-lib';
import { Template, Match } from 'aws-cdk-lib/assertions';
import { GoldskyStack } from '../../lib/goldsky-stack';

function synth() {
  const app = new cdk.App();
  const stack = new GoldskyStack(app, 'TestStack');
  return Template.fromStack(stack);
}

describe('GoldskyStack', () => {
  test('creates two Lambda functions (http + periodic)', () => {
    synth().resourceCountIs('AWS::Lambda::Function', 2);
  });

  test('wires an EventBridge rule on a 1-minute schedule', () => {
    synth().hasResourceProperties('AWS::Events::Rule', {
      ScheduleExpression: 'rate(1 minute)',
    });
  });

  test('exposes an API Gateway with CORS on the registry/contracts resource', () => {
    const t = synth();
    t.resourceCountIs('AWS::ApiGateway::RestApi', 1);
    t.hasResourceProperties('AWS::ApiGateway::Method', {
      HttpMethod: 'GET',
    });
    t.hasResourceProperties('AWS::ApiGateway::Method', {
      HttpMethod: 'OPTIONS',
    });
  });

  test('grants both lambdas read access to the goldsky-pg-url secret', () => {
    const statements = synth().findResources('AWS::IAM::Policy');
    const serialized = JSON.stringify(statements);
    expect(serialized).toContain('secretsmanager:GetSecretValue');
    expect(serialized).toContain('goldsky-pg-url');
  });

  test('throttles the api deployment to 1 rps / 1 burst', () => {
    synth().hasResourceProperties('AWS::ApiGateway::Stage', {
      MethodSettings: Match.arrayWith([
        Match.objectLike({ ThrottlingBurstLimit: 1, ThrottlingRateLimit: 1 }),
      ]),
    });
  });
});
