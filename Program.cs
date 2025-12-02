using DotnetEmail;
using Google.Apis.Gmail.v1;
using Google.Apis.Gmail.v1.Data;

Console.WriteLine("Gmail API .NET Quickstart");

try
{
    var service = await GmailServiceHelper.GetGmailService();

    // Define parameters of request.
    UsersResource.MessagesResource.ListRequest request = service.Users.Messages.List("me");
    request.MaxResults = 10;
    request.LabelIds = new List<string> { "INBOX" };
    request.IncludeSpamTrash = false;

    // List messages.
    ListMessagesResponse response = await request.ExecuteAsync();
    IList<Message> messages = response.Messages;
    Console.WriteLine("Messages:");
    if (messages != null && messages.Count > 0)
    {
        foreach (var messageItem in messages)
        {
            var msgRequest = service.Users.Messages.Get("me", messageItem.Id);
            var message = await msgRequest.ExecuteAsync();
            var dateTime = ConvertUnixEpochToDateTime(message.InternalDate);
            Console.WriteLine($"- {dateTime?.ToLocalTime()}: {message.Snippet} (ID: {message.Id})");
        }
    }
    else
    {
        Console.WriteLine("No messages found.");
    }
}
catch (FileNotFoundException)
{
    Console.WriteLine("Error: credentials.json not found. Please download it from Google Cloud Console and place it in the project directory.");
}
catch (Exception e)
{
    Console.WriteLine("An error occurred: " + e.Message);
}

static DateTimeOffset? ConvertUnixEpochToDateTime(long? epochMs)
{
    return epochMs.HasValue ? DateTimeOffset.FromUnixTimeMilliseconds(epochMs.Value) : null;
}
