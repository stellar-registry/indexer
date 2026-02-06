import { Handler } from "aws-cdk-lib/aws-lambda";

export const handler: Handler = async (event: any) => {
  console.log('event: ', JSON.stringify(event))
  return {
    statusCode: 200,
    body: JSON.stringify({ message: "Hello from TS lambda v2" })
  };
};
